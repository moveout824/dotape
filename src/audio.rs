use crate::model::{PEAK_BLOCK, Peak, Project, Source, db_gain};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::RingBuffer;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone)]
pub struct InputDevice {
    pub name: String,
    pub device: cpal::Device,
    pub channels: u16,
}

pub fn input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    let Ok(devices) = host.input_devices() else {
        return vec![];
    };
    let mut devices: Vec<_> = devices
        .filter_map(|device| {
            let name = device.name().ok()?;
            let channels = device.default_input_config().ok()?.channels();
            Some(InputDevice {
                name,
                device,
                channels,
            })
        })
        .collect();
    devices.sort_by_key(|d| {
        if Some(&d.name) == default_name.as_ref() {
            0
        } else {
            1
        }
    });
    devices
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputMode {
    Left,
    Right,
    Mix,
    Stereo,
}
impl InputMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "모노 · 입력 1",
            Self::Right => "모노 · 입력 2",
            Self::Mix => "모노 · 입력 합치기",
            Self::Stereo => "스테레오 · 입력 1 / 2",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Left => {
                "마이크 하나로 목소리를 녹음할 때 추천합니다. 기기의 첫 번째 입력을 양쪽 스피커 중앙에서 들리게 녹음합니다."
            }
            Self::Right => {
                "오디오 인터페이스의 두 번째 입력에 마이크를 연결했다면 선택하세요. 입력이 하나인 기기에서는 사용할 수 없습니다."
            }
            Self::Mix => {
                "여러 입력을 평균 내어 모노로 합칩니다. 빈 입력이 섞이면 목소리가 작아질 수 있어 마이크 하나에는 ‘입력 1’을 추천합니다."
            }
            Self::Stereo => {
                "입력 1을 왼쪽, 입력 2를 오른쪽에 보존합니다. 스테레오 마이크나 악기에 적합합니다. 마이크 하나에는 모노를 쓰세요."
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecordSettings {
    pub rate: u32,
    pub mode: InputMode,
    pub gain_db: f32,
    pub buffer: u32,
    pub overdub: bool,
}
impl Default for RecordSettings {
    fn default() -> Self {
        Self {
            rate: 0,
            mode: InputMode::Left,
            gain_db: 0.0,
            buffer: 0,
            overdub: false,
        }
    }
}

#[derive(Default)]
pub struct LiveState {
    pub peaks: Vec<Peak>,
}

pub struct Recorder {
    stream: Option<cpal::Stream>,
    worker: Option<JoinHandle<Result<Source, String>>>,
    stop: Arc<AtomicBool>,
    pub frames: Arc<AtomicU64>,
    pub dropped: Arc<AtomicU64>,
    pub meter: Arc<AtomicU32>,
    pub error: Arc<Mutex<Option<String>>>,
    pub live: Arc<Mutex<LiveState>>,
    pub rate: u32,
}

fn select_input_config(
    device: &cpal::Device,
    requested: u32,
    buffer: u32,
) -> Result<(cpal::StreamConfig, cpal::SampleFormat), String> {
    let default = device
        .default_input_config()
        .map_err(|e| format!("마이크 설정을 읽지 못했습니다: {e}"))?;
    let chosen = if requested == 0 || requested == default.sample_rate().0 {
        default
    } else {
        device.supported_input_configs().map_err(|e| e.to_string())?
            .filter(|c| c.min_sample_rate().0 <= requested && c.max_sample_rate().0 >= requested)
            .max_by_key(|c| (c.sample_format() == cpal::SampleFormat::F32, c.channels() == default.channels()))
            .map(|c| c.with_sample_rate(cpal::SampleRate(requested)))
            .ok_or_else(|| format!("이 마이크는 {requested} Hz 입력을 지원하지 않습니다. 녹음 샘플레이트를 ‘기기 자동’으로 바꾸세요. 내보내기 샘플레이트는 따로 고를 수 있습니다."))?
    };
    let format = chosen.sample_format();
    let buffer_ok = match chosen.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => buffer >= *min && buffer <= *max,
        cpal::SupportedBufferSize::Unknown => true,
    };
    if buffer != 0 && !buffer_ok {
        return Err("이 기기가 지원하지 않는 버퍼 크기입니다. ‘기기 자동’을 선택하세요.".into());
    }
    let mut config = chosen.config();
    if buffer != 0 {
        config.buffer_size = cpal::BufferSize::Fixed(buffer);
    }
    Ok((config, format))
}

impl Recorder {
    pub fn start(
        input: &InputDevice,
        settings: &RecordSettings,
        source_id: u64,
        path: &Path,
    ) -> Result<Self, String> {
        let (config, format) = select_input_config(&input.device, settings.rate, settings.buffer)?;
        if matches!(settings.mode, InputMode::Right | InputMode::Stereo) && config.channels < 2 {
            return Err("입력이 하나인 마이크입니다. ‘모노 · 입력 1’을 선택하세요.".into());
        }
        let rate = config.sample_rate.0;
        let channels = if settings.mode == InputMode::Stereo {
            2
        } else {
            1
        };
        let (producer, mut consumer) = RingBuffer::<[f32; 2]>::new(rate as usize * 4);
        let frames = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let meter = Arc::new(AtomicU32::new(0.0_f32.to_bits()));
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let live = Arc::new(Mutex::new(LiveState::default()));
        macro_rules! build {
            ($ty:ty) => {
                build_input::<$ty>(
                    &input.device,
                    &config,
                    producer,
                    settings.mode,
                    db_gain(settings.gain_db),
                    dropped.clone(),
                    error.clone(),
                )
            };
        }
        let stream = match format {
            cpal::SampleFormat::F32 => build!(f32), cpal::SampleFormat::F64 => build!(f64),
            cpal::SampleFormat::I16 => build!(i16), cpal::SampleFormat::I32 => build!(i32),
            cpal::SampleFormat::I8 => build!(i8), cpal::SampleFormat::I64 => build!(i64),
            cpal::SampleFormat::U8 => build!(u8), cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::U32 => build!(u32), cpal::SampleFormat::U64 => build!(u64),
            _ => return Err(format!("지원하지 않는 마이크 샘플 형식입니다: {format}")),
        }.map_err(|e| format!("마이크를 열지 못했습니다. 다른 앱이 독점 사용 중인지, Windows 마이크 접근이 허용되어 있는지 확인하세요.\n{e}"))?;
        let spec = hound::WavSpec {
            channels: channels as u16,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::new(
            BufWriter::new(
                File::create(path)
                    .map_err(|e| format!("녹음 복구 파일을 만들지 못했습니다: {e}"))?,
            ),
            spec,
        )
        .map_err(|e| e.to_string())?;
        let saved_path = path.to_string_lossy().to_string();
        let (stop_worker, frames_worker, meter_worker, live_worker) =
            (stop.clone(), frames.clone(), meter.clone(), live.clone());
        let worker = thread::spawn(move || {
            let mut samples = Vec::with_capacity(rate as usize * channels * 10);
            let mut peak = Peak::default();
            let mut peak_frames = 0;
            let mut last_flush = 0;
            loop {
                let mut pending = Vec::new();
                let mut batch_peak = 0.0_f32;
                while let Ok(frame) = consumer.pop() {
                    for value in frame.iter().take(channels) {
                        writer
                            .write_sample(*value)
                            .map_err(|e| format!("녹음 파일 쓰기 실패: {e}"))?;
                        samples.push(*value);
                        peak.min = peak.min.min(*value);
                        peak.max = peak.max.max(*value);
                        batch_peak = batch_peak.max(value.abs());
                    }
                    peak_frames += 1;
                    if peak_frames == PEAK_BLOCK {
                        pending.push(peak);
                        peak = Peak::default();
                        peak_frames = 0;
                    }
                }
                frames_worker.store((samples.len() / channels) as u64, Ordering::Release);
                if samples.len() / channels >= last_flush + rate as usize {
                    writer
                        .flush()
                        .map_err(|e| format!("녹음 파일 저장 실패: {e}"))?;
                    last_flush = samples.len() / channels;
                }
                meter_worker.store(batch_peak.to_bits(), Ordering::Relaxed);
                if !pending.is_empty() {
                    if let Ok(mut live) = live_worker.lock() {
                        live.peaks.extend(pending);
                    }
                }
                if stop_worker.load(Ordering::Acquire) && consumer.is_empty() {
                    break;
                }
                thread::sleep(Duration::from_millis(6));
            }
            writer
                .finalize()
                .map_err(|e| format!("녹음 파일 저장 실패: {e}"))?;
            Ok(Source::new(source_id, samples, rate, channels, saved_path))
        });
        if let Err(e) = stream.play() {
            stop.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(format!("마이크를 시작하지 못했습니다: {e}"));
        }
        Ok(Self {
            stream: Some(stream),
            worker: Some(worker),
            stop,
            frames,
            dropped,
            meter,
            error,
            live,
            rate,
        })
    }
    pub fn seconds(&self) -> f64 {
        self.frames.load(Ordering::Acquire) as f64 / self.rate as f64
    }
    pub fn worker_finished(&self) -> bool {
        self.worker.as_ref().is_some_and(|w| w.is_finished())
    }
    pub fn stop(mut self) -> Result<Source, String> {
        self.stream.take();
        self.stop.store(true, Ordering::Release);
        self.worker
            .take()
            .ok_or("녹음 작업이 없습니다.")?
            .join()
            .map_err(|_| "녹음 처리 중 오류가 발생했습니다.".to_string())?
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stream.take();
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut producer: rtrb::Producer<[f32; 2]>,
    mode: InputMode,
    gain: f32,
    dropped: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            for frame in data.chunks_exact(channels) {
                let get = |n: usize| {
                    let v = <f32 as cpal::Sample>::from_sample(frame[n]);
                    if v.is_finite() { v * gain } else { 0.0 }
                };
                let pair = match mode {
                    InputMode::Left => [get(0), get(0)],
                    InputMode::Right => [get(1), get(1)],
                    InputMode::Stereo => [get(0), get(1)],
                    InputMode::Mix => {
                        let v = (0..channels).map(get).sum::<f32>() / channels as f32;
                        [v, v]
                    }
                };
                if producer.push(pair).is_err() {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        },
        move |e| {
            if let Ok(mut error) = error.lock() {
                *error = Some(e.to_string());
            }
        },
        None,
    )
}

pub struct Playback {
    _stream: cpal::Stream,
    pub position: Arc<AtomicU64>,
    pub finished: Arc<AtomicBool>,
    pub meter: Arc<AtomicU32>,
    pub error: Arc<Mutex<Option<String>>>,
    rate: u32,
    start: f64,
}
impl Playback {
    pub fn start(
        project: Arc<Project>,
        start: f64,
        continue_silence: bool,
    ) -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("재생할 출력 장치를 찾지 못했습니다.")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate.0;
        let position = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let meter = Arc::new(AtomicU32::new(0));
        let error = Arc::new(Mutex::new(None));
        macro_rules! build {
            ($ty:ty) => {
                build_output::<$ty>(
                    &device,
                    &config,
                    project,
                    start,
                    continue_silence,
                    position.clone(),
                    finished.clone(),
                    meter.clone(),
                    error.clone(),
                )
            };
        }
        let stream = match format {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::F64 => build!(f64),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::I32 => build!(i32),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::U8 => build!(u8),
            _ => return Err(format!("지원하지 않는 출력 형식입니다: {format}")),
        }
        .map_err(|e| format!("재생 장치를 열지 못했습니다: {e}"))?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            position,
            finished,
            meter,
            error,
            rate,
            start,
        })
    }
    pub fn seconds(&self) -> f64 {
        self.start + self.position.load(Ordering::Relaxed) as f64 / self.rate as f64
    }
}

#[allow(clippy::too_many_arguments)]
fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    project: Arc<Project>,
    start: f64,
    continue_silence: bool,
    position: Arc<AtomicU64>,
    finished: Arc<AtomicBool>,
    meter: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f64;
    let end = project.duration();
    let mut rendered = 0_u64;
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut peak = 0.0_f32;
            for frame in data.chunks_exact_mut(channels) {
                let t = start + rendered as f64 / rate;
                let value = project.mix_frame(t);
                peak = peak.max(value[0].abs()).max(value[1].abs());
                for (c, out) in frame.iter_mut().enumerate() {
                    let v = if channels == 1 {
                        (value[0] + value[1]) * 0.5
                    } else if c < 2 {
                        value[c]
                    } else {
                        0.0
                    };
                    *out = T::from_sample(v.clamp(-1.0, 1.0));
                }
                rendered += 1;
                if t >= end && !continue_silence {
                    finished.store(true, Ordering::Relaxed);
                }
            }
            meter.store(peak.to_bits(), Ordering::Relaxed);
            position.store(rendered, Ordering::Relaxed);
        },
        move |e| {
            if let Ok(mut error) = error.lock() {
                *error = Some(e.to_string());
            }
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Opens default audio devices for a local 300 ms smoke test"]
    fn real_audio_input_and_silent_output_callbacks() {
        let devices = input_devices();
        let input = devices.first().expect("No input device");
        let root = std::env::var_os("SORI_TEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("Sori-tests"));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("hardware-input.wav");
        let r = Recorder::start(input, &RecordSettings::default(), 1, &path).unwrap();
        thread::sleep(Duration::from_millis(300));
        let source = r.stop().unwrap();
        assert!(source.frames() > 0);
        assert!(source.duration() > 0.1);
        let reader = hound::WavReader::open(path).unwrap();
        assert_eq!(reader.duration() as usize, source.frames());
        let mut project = Project::default();
        project.add_source(
            Source::new(2, vec![0.0; 4800], 48000, 1, String::new()),
            3,
            0,
            0.0,
        );
        let p = Playback::start(Arc::new(project), 0.0, false).unwrap();
        thread::sleep(Duration::from_millis(300));
        assert!(p.position.load(Ordering::Relaxed) > 0);
        assert!(p.finished.load(Ordering::Relaxed));
    }
}
