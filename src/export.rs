use crate::model::{Project, Source};
use rubato::{FftFixedInOut, Resampler};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Seek, SeekFrom, Write},
    num::{NonZeroU8, NonZeroU32},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
pub enum Format {
    Wav,
    Mp3,
    Flac,
    Ogg,
}
impl Format {
    pub const ALL: [Self; 4] = [Self::Wav, Self::Mp3, Self::Flac, Self::Ogg];
    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Mp3 => "MP3",
            Self::Flac => "FLAC",
            Self::Ogg => "OGG",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Wav => {
                "편집용 추천. 압축하지 않아 원음을 보존하고 대부분의 편집 프로그램에서 열립니다. 파일 크기는 큽니다."
            }
            Self::Mp3 => {
                "공유용 추천. 휴대폰과 대부분의 플레이어에서 열리고 파일이 작습니다. 손실 압축이므로 편집 원본 보관에는 WAV나 FLAC이 좋습니다."
            }
            Self::Flac => {
                "보관용 추천. 소리 손실 없이 WAV보다 작게 저장합니다. 일부 편집 앱에서는 열리지 않을 수 있습니다."
            }
            Self::Ogg => {
                "작은 파일로 좋은 음질이 필요할 때. Ogg Vorbis로 저장합니다. 일부 기기나 서비스는 지원하지 않습니다."
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub format: Format,
    pub rate: u32,
    pub bits: u16,
    pub channels: usize,
    pub bitrate: u32,
    pub normalize: bool,
    pub dither: bool,
    /// Legacy field name retained for saved sessions: true now exports one OTIOZ project.
    #[serde(default)]
    pub split_tracks: bool,
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: Format::Wav,
            rate: 48000,
            bits: 24,
            channels: 2,
            bitrate: 192,
            normalize: false,
            dither: true,
            split_tracks: false,
        }
    }
}
impl ExportSettings {
    pub fn output_label(&self) -> &'static str {
        if self.split_tracks {
            "OTIOZ"
        } else {
            self.format.label()
        }
    }
    pub fn output_extension(&self) -> &'static str {
        if self.split_tracks {
            "otioz"
        } else {
            self.format.extension()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if ![44100, 48000, 96000].contains(&self.rate)
            || ![1, 2].contains(&self.channels)
            || ![16, 24, 32].contains(&self.bits)
            || ![128, 192, 256, 320].contains(&self.bitrate)
        {
            return Err("지원하지 않는 출력 설정입니다.".into());
        }
        if self.format == Format::Mp3 && self.rate > 48000 {
            return Err("MP3는 최대 48 kHz로 저장할 수 있습니다.".into());
        }
        if self.format == Format::Flac && self.bits == 32 {
            return Err("FLAC은 16 또는 24비트를 선택하세요.".into());
        }
        Ok(())
    }
}

pub(crate) fn resampled_project(project: &Project, rate: u32) -> Result<Project, String> {
    let mut prepared = project.clone();
    prepared
        .sources
        .retain(|id, _| project.clips.iter().any(|c| c.source == *id));
    for source in prepared.sources.values_mut() {
        if source.rate == rate {
            continue;
        }
        let mut converter =
            FftFixedInOut::<f32>::new(source.rate as usize, rate as usize, 2048, source.channels)
                .map_err(|e| e.to_string())?;
        let input_n = converter.input_frames_next();
        let output_n = converter.output_frames_next();
        let delay = converter.output_delay();
        let target_n = (source.frames() as f64 * rate as f64 / source.rate as f64).round() as usize;
        let mut planar = vec![Vec::new(); source.channels];
        for channel in &mut planar {
            channel.try_reserve_exact(target_n + delay + output_n).map_err(|_| "샘플레이트 변환에 필요한 메모리가 부족합니다. 녹음과 같은 샘플레이트로 내보내세요.".to_string())?;
        }
        let mut at = 0;
        while planar[0].len() < target_n + delay {
            let mut block = vec![vec![0.0_f32; input_n]; source.channels];
            let available = source.frames().saturating_sub(at).min(input_n);
            for (c, plane) in block.iter_mut().enumerate() {
                for (i, sample) in plane.iter_mut().take(available).enumerate() {
                    *sample = source.samples[(at + i) * source.channels + c];
                }
            }
            let out = converter.process(&block, None).map_err(|e| e.to_string())?;
            for (dst, data) in planar.iter_mut().zip(out) {
                dst.extend(data);
            }
            at += available;
        }
        let mut interleaved = Vec::new();
        interleaved
            .try_reserve_exact(target_n * source.channels)
            .map_err(|_| "샘플레이트 변환 메모리가 부족합니다.".to_string())?;
        for i in delay..delay + target_n {
            for plane in &planar {
                interleaved.push(plane[i]);
            }
        }
        *source = Arc::new(Source::new(
            source.id,
            interleaved,
            rate,
            source.channels,
            source.path.clone(),
        ));
    }
    Ok(prepared)
}

struct Render<'a> {
    project: &'a Project,
    settings: &'a ExportSettings,
    frame: usize,
    total: usize,
    scale: f32,
    rng: u32,
    progress: Arc<AtomicU32>,
}
impl Render<'_> {
    fn block(&mut self, frames: usize) -> Vec<Vec<f32>> {
        let count = frames.min(self.total.saturating_sub(self.frame));
        let mut planes = vec![Vec::with_capacity(count); self.settings.channels];
        for n in self.frame..self.frame + count {
            let [l, r] = self.project.mix_frame(n as f64 / self.settings.rate as f64);
            if self.settings.channels == 1 {
                planes[0].push((l + r) * 0.5 * self.scale);
            } else {
                planes[0].push(l * self.scale);
                planes[1].push(r * self.scale);
            }
        }
        self.frame += count;
        self.progress.store(
            (10.0 + 85.0 * self.frame as f64 / self.total.max(1) as f64) as u32,
            Ordering::Relaxed,
        );
        planes
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
    fn quantize(&mut self, v: f32) -> i32 {
        let maximum = (1_i64 << (self.settings.bits - 1)) - 1;
        let dither = if self.settings.dither {
            self.random() - self.random()
        } else {
            0.0
        };
        (v as f64 * maximum as f64 + dither as f64)
            .round()
            .clamp(-(maximum as f64) - 1.0, maximum as f64) as i32
    }
}

impl flacenc::source::Source for Render<'_> {
    fn channels(&self) -> usize {
        self.settings.channels
    }
    fn bits_per_sample(&self) -> usize {
        self.settings.bits as usize
    }
    fn sample_rate(&self) -> usize {
        self.settings.rate as usize
    }
    fn len_hint(&self) -> Option<usize> {
        Some(self.total)
    }
    fn read_samples<F: flacenc::source::Fill>(
        &mut self,
        block_size: usize,
        dest: &mut F,
    ) -> Result<usize, flacenc::error::SourceError> {
        let planes = self.block(block_size);
        let n = planes[0].len();
        let mut pcm = Vec::with_capacity(n * planes.len());
        for i in 0..n {
            for plane in &planes {
                pcm.push(self.quantize(plane[i]));
            }
        }
        dest.fill_interleaved(&pcm)?;
        Ok(n)
    }
}

pub struct ExportResult {
    pub path: PathBuf,
    pub clipped: bool,
}

pub fn export(
    project: &Project,
    settings: &ExportSettings,
    path: &Path,
    progress: Arc<AtomicU32>,
) -> Result<ExportResult, String> {
    if settings.split_tracks {
        return crate::interchange::export_project(project, settings, path, progress);
    }
    export_single(project, settings, path, progress, None, None)
}

pub(crate) fn export_single(
    project: &Project,
    settings: &ExportSettings,
    path: &Path,
    progress: Arc<AtomicU32>,
    total_frames: Option<usize>,
    shared_scale: Option<f32>,
) -> Result<ExportResult, String> {
    settings.validate()?;
    if project.clips.is_empty() {
        return Err("녹음한 클립이 없습니다.".into());
    }
    let prepared = resampled_project(project, settings.rate)?;
    let total =
        total_frames.unwrap_or_else(|| (project.duration() * settings.rate as f64).ceil() as usize);
    // RIFF WAV has a 4 GiB data-size limit. Refuse before creating a partial file.
    if settings.format == Format::Wav
        && total as u64 * settings.channels as u64 * (settings.bits as u64 / 8) > 4_294_967_000
    {
        return Err(
            "WAV의 4 GB 한도를 넘습니다. FLAC을 고르거나 길이·샘플레이트를 줄여주세요.".into(),
        );
    }
    let mut peak = 0.0_f32;
    for i in 0..total {
        let value = prepared.mix_frame(i as f64 / settings.rate as f64);
        let p = if settings.channels == 1 {
            ((value[0] + value[1]) * 0.5).abs()
        } else {
            value[0].abs().max(value[1].abs())
        };
        peak = peak.max(p);
        if i % 8192 == 0 {
            progress.store(
                (i as f64 / total.max(1) as f64 * 10.0) as u32,
                Ordering::Relaxed,
            );
        }
    }
    let scale = shared_scale.unwrap_or_else(|| {
        if settings.normalize && peak > 0.0 {
            0.891_250_9 / peak
        } else {
            1.0
        }
    });
    let mut render = Render {
        project: &prepared,
        settings,
        frame: 0,
        total,
        scale,
        rng: 0x72a4_3b91,
        progress: progress.clone(),
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = path.with_file_name(format!(".sori-export-{nonce}.part"));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|e| format!("저장할 위치를 열지 못했습니다: {e}"))?;
    let result = encode(file, &mut render);
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("완성 파일을 저장하지 못했습니다: {e}"));
    }
    progress.store(100, Ordering::Relaxed);
    Ok(ExportResult {
        path: path.to_path_buf(),
        clipped: peak * scale > 1.0 && !(settings.format == Format::Wav && settings.bits == 32),
    })
}

pub fn safe_name(name: &str) -> String {
    let text: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(64)
        .collect();
    let text = text.trim_end_matches([' ', '.']);
    if text.is_empty() {
        "녹음".into()
    } else {
        text.into()
    }
}

fn encode(file: File, render: &mut Render<'_>) -> Result<(), String> {
    let s = render.settings;
    match s.format {
        Format::Wav => {
            let spec = hound::WavSpec {
                channels: s.channels as u16,
                sample_rate: s.rate,
                bits_per_sample: s.bits,
                sample_format: if s.bits == 32 {
                    hound::SampleFormat::Float
                } else {
                    hound::SampleFormat::Int
                },
            };
            let mut writer =
                hound::WavWriter::new(BufWriter::new(file), spec).map_err(|e| e.to_string())?;
            while render.frame < render.total {
                let block = render.block(4096);
                for i in 0..block[0].len() {
                    for channel in &block {
                        if s.bits == 32 {
                            writer.write_sample(channel[i]).map_err(|e| e.to_string())?;
                        } else {
                            writer
                                .write_sample(render.quantize(channel[i]))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            writer.finalize().map_err(|e| e.to_string())?;
        }
        Format::Flac => {
            use flacenc::{component::BitRepr, error::Verify};
            let config = flacenc::config::Encoder::default()
                .into_verified()
                .map_err(|e| format!("FLAC 설정 오류: {e:?}"))?;
            let stream = flacenc::encode_with_fixed_block_size(&config, render, config.block_size)
                .map_err(|e| e.to_string())?;
            let mut sink = flacenc::bitsink::ByteSink::new();
            stream.write(&mut sink).map_err(|e| e.to_string())?;
            let mut file = BufWriter::new(file);
            file.write_all(sink.as_slice()).map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
        }
        Format::Mp3 => {
            use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushGap, MonoPcm, Quality};
            let brate = match s.bitrate {
                128 => Bitrate::Kbps128,
                256 => Bitrate::Kbps256,
                320 => Bitrate::Kbps320,
                _ => Bitrate::Kbps192,
            };
            let mut builder = Builder::new().ok_or("MP3 인코더를 만들지 못했습니다.")?;
            builder
                .set_num_channels(s.channels as u8)
                .map_err(|e| e.to_string())?;
            builder.set_sample_rate(s.rate).map_err(|e| e.to_string())?;
            builder.set_brate(brate).map_err(|e| e.to_string())?;
            builder
                .set_quality(Quality::Good)
                .map_err(|e| e.to_string())?;
            builder
                .set_to_write_vbr_tag(true)
                .map_err(|e| e.to_string())?;
            let mut encoder = builder.build().map_err(|e| e.to_string())?;
            let mut writer = BufWriter::new(file);
            let mut bytes = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(4096));
            while render.frame < render.total {
                let block = render.block(4096);
                bytes.clear();
                if s.channels == 1 {
                    encoder
                        .encode_to_vec(MonoPcm(&block[0]), &mut bytes)
                        .map_err(|e| e.to_string())?;
                } else {
                    encoder
                        .encode_to_vec(
                            DualPcm {
                                left: &block[0],
                                right: &block[1],
                            },
                            &mut bytes,
                        )
                        .map_err(|e| e.to_string())?;
                }
                writer.write_all(&bytes).map_err(|e| e.to_string())?;
            }
            bytes.clear();
            encoder
                .flush_to_vec::<FlushGap>(&mut bytes)
                .map_err(|e| e.to_string())?;
            writer.write_all(&bytes).map_err(|e| e.to_string())?;
            let mut tag = Vec::with_capacity(encoder.lame_tag_size());
            if encoder.lame_tag_encode_to_vec(&mut tag).is_some() {
                writer
                    .seek(SeekFrom::Start(encoder.id3v2_tag_size() as u64))
                    .map_err(|e| e.to_string())?;
                writer.write_all(&tag).map_err(|e| e.to_string())?;
            }
            writer.flush().map_err(|e| e.to_string())?;
        }
        Format::Ogg => {
            let mut builder = vorbis_rs::VorbisEncoderBuilder::new_with_serial(
                NonZeroU32::new(s.rate).unwrap(),
                NonZeroU8::new(s.channels as u8).unwrap(),
                BufWriter::new(file),
                0x534f5249,
            );
            builder.bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::Vbr {
                target_bitrate: NonZeroU32::new(s.bitrate * 1000).unwrap(),
            });
            let mut encoder = builder.build().map_err(|e| e.to_string())?;
            while render.frame < render.total {
                let block = render.block(4096);
                encoder
                    .encode_audio_block(&block)
                    .map_err(|e| e.to_string())?;
            }
            let mut writer = encoder.finish().map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn demo_project() -> Project {
    let mut project = Project::default();
    project.name = "겹치고, 자르고, 자연스럽게".into();
    project.add_track();
    project.add_track();
    project.add_track();
    let rate = 48000;
    for (n, (start, length, freq)) in [(0.6, 6.4, 220.0), (3.2, 5.0, 330.0), (6.4, 4.1, 165.0)]
        .into_iter()
        .enumerate()
    {
        let samples = (0..(length * rate as f64) as usize)
            .map(|i| {
                let t = i as f64 / rate as f64;
                let shape = (t * 4.6).sin().abs().powf(2.0) * (0.6 + 0.4 * (t * 8.5).sin().abs());
                ((t * freq * std::f64::consts::TAU).sin() * shape * 0.44) as f32
            })
            .collect();
        project.add_source(
            Source::new(n as u64 + 1, samples, rate, 1, String::new()),
            n as u64 + 10,
            n,
            start,
        );
        let c = project.clips.last_mut().unwrap();
        c.fade_in = 0.45;
        c.fade_out = 0.8;
    }
    project.clips[0].name = "첫 번째 목소리".into();
    project.clips[1].name = "겹쳐 녹음하기".into();
    project.clips[2].name = "부드러운 마무리".into();
    project
}

pub fn self_test() -> Result<String, String> {
    let folder = std::env::var_os("SORI_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("Sori-self-test"));
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let p = demo_project();
    for format in Format::ALL {
        for channels in [1, 2] {
            let s = ExportSettings {
                format,
                channels,
                rate: 44100,
                bits: 24,
                dither: false,
                ..Default::default()
            };
            let path = folder.join(format!("mix-{channels}.{}", format.extension()));
            export(&p, &s, &path, Arc::new(AtomicU32::new(0)))?;
        }
    }
    let s = ExportSettings {
        bits: 32,
        dither: false,
        ..Default::default()
    };
    let path = folder.join("reference.wav");
    export(&p, &s, &path, Arc::new(AtomicU32::new(0)))?;
    let mut wav = hound::WavReader::open(&path).map_err(|e| e.to_string())?;
    for (i, sample) in wav.samples::<f32>().enumerate() {
        let sample = sample.map_err(|e| e.to_string())?;
        let expected = p.mix_frame((i / 2) as f64 / s.rate as f64)[i % 2];
        if (sample - expected).abs() > 0.000001 {
            return Err(format!("PCM 비교 실패: sample {i}"));
        }
    }
    export(
        &p,
        &ExportSettings {
            split_tracks: true,
            ..Default::default()
        },
        &folder.join("timeline.otioz"),
        Arc::new(AtomicU32::new(0)),
    )?;
    Ok(format!(
        "PASS: 9 audio exports + 1 OTIOZ project; float WAV matches the timeline sample-for-sample. {}",
        folder.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fft_resampling_keeps_length_and_alignment() {
        let mut p = Project::default();
        let mut samples = vec![0.0; 48000];
        samples[24000] = 0.5;
        p.add_source(Source::new(1, samples, 48000, 1, String::new()), 2, 0, 0.0);
        let out = resampled_project(&p, 44100).unwrap();
        let source = &out.sources[&1];
        assert_eq!(source.frames(), 44100);
        let max = source
            .samples
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert!(max.abs_diff(22050) <= 1);
    }
    #[test]
    fn invalid_export_settings_are_rejected() {
        assert!(
            ExportSettings {
                format: Format::Mp3,
                rate: 96000,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            ExportSettings {
                channels: 5,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
