//! Self-contained Resolve timeline: standard OTIOZ, written natively in Rust.
//! Gain and fades are baked into each clip's float WAV. Edits and lanes remain separate.

use crate::{
    export::{self, ExportResult, ExportSettings, Format},
    model::Project,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn time(value: u64, rate: u32) -> Value {
    json!({"OTIO_SCHEMA":"RationalTime.1", "value":value, "rate":rate})
}
fn range(start: u64, duration: u64, rate: u32) -> Value {
    json!({"OTIO_SCHEMA":"TimeRange.1", "start_time":time(start,rate), "duration":time(duration,rate)})
}
fn gap(frames: u64, rate: u32) -> Value {
    json!({"OTIO_SCHEMA":"Gap.1", "name":"", "metadata":{}, "source_range":range(0,frames,rate), "effects":[], "markers":[]})
}

struct Staging {
    path: std::path::PathBuf,
    parent: std::path::PathBuf,
}
impl Drop for Staging {
    fn drop(&mut self) {
        if self
            .path
            .canonicalize()
            .is_ok_and(|path| path.parent() == Some(&self.parent))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub fn export_project(
    project: &Project,
    settings: &ExportSettings,
    path: &Path,
    progress: Arc<AtomicU32>,
) -> Result<ExportResult, String> {
    // Project mode always uses lossless float audio. Previous audio-file options are untouched.
    let audio_settings = ExportSettings {
        format: Format::Wav,
        bits: 32,
        normalize: false,
        dither: false,
        split_tracks: false,
        ..settings.clone()
    };
    audio_settings.validate()?;
    if project.clips.is_empty() {
        return Err("녹음한 클립이 없습니다.".into());
    }
    if project.clips.iter().any(|c| {
        !c.start.is_finite()
            || c.start < 0.0
            || !c.len.is_finite()
            || c.len <= 0.0
            || !project.sources.contains_key(&c.source)
            || c.track >= project.tracks.len()
    }) {
        return Err("클립의 원본이나 배치를 확인해 주세요.".into());
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .map_err(|e| format!("저장 폴더를 열지 못했습니다: {e}"))?;
    let stage = parent.join(format!(".dotape-project-{nonce}.tmp"));
    fs::create_dir(&stage).map_err(|e| format!("프로젝트를 저장할 위치를 열지 못했습니다: {e}"))?;
    // The guard owns only the fresh create_dir above, never a user-supplied directory.
    let stage = Staging {
        path: stage,
        parent,
    };
    let prepared = export::resampled_project(project, settings.rate)?;
    let rate = settings.rate;
    let total = (prepared.duration() * rate as f64).ceil() as u64;
    let scale = if settings.normalize {
        let mut peak = 0.0_f32;
        for frame in 0..total {
            let [l, r] = prepared.mix_frame(frame as f64 / rate as f64);
            peak = peak.max(if settings.channels == 1 {
                ((l + r) * 0.5).abs()
            } else {
                l.abs().max(r.abs())
            });
            if frame % 8192 == 0 {
                progress.store(
                    (frame as f64 / total.max(1) as f64 * 10.0) as u32,
                    Ordering::Relaxed,
                );
            }
        }
        if peak > 0.0 { 0.891_250_9 / peak } else { 1.0 }
    } else {
        1.0
    };
    let bundle_path = stage.path.join("bundle.part");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&bundle_path)
        .map_err(|e| e.to_string())?;
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    archive
        .start_file("version.txt", options)
        .map_err(|e| e.to_string())?;
    archive.write_all(b"1.0.0").map_err(|e| e.to_string())?;
    let mut tracks = Vec::new();
    let mut rendered = 0;
    for (track_index, track) in prepared.tracks.iter().enumerate() {
        let mut clips: Vec<_> = prepared
            .clips
            .iter()
            .filter(|c| c.track == track_index)
            .collect();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        if clips.is_empty() {
            continue;
        }
        let mut children = Vec::new();
        let mut cursor = 0;
        for clip in clips {
            let start = (clip.start * rate as f64).ceil() as u64;
            let end = (clip.end() * rate as f64).ceil() as u64;
            if start < cursor {
                return Err("같은 트랙에 겹친 클립이 있습니다. 배치를 확인해 주세요.".into());
            }
            if end <= start {
                return Err("샘플 한 개보다 짧은 클립은 저장할 수 없습니다.".into());
            }
            if start > cursor {
                children.push(gap(start - cursor, rate));
            }
            let frames = end - start;
            let mut single = prepared.clone();
            single.clips = vec![clip.clone()];
            let delta = start as f64 / rate as f64 - clip.start;
            single.clips[0].start = 0.0;
            single.clips[0].offset += delta;
            single.clips[0].envelope_offset += delta;
            single.clips[0].len = frames as f64 / rate as f64;
            // Keep muted media recoverable; OTIO's enabled state carries its mute.
            single.tracks[track_index].muted = false;
            let media_name = format!("media/clip-{:06}.wav", rendered + 1);
            let media_path = stage.path.join("clip.wav");
            export::export_single(
                &single,
                &audio_settings,
                &media_path,
                Arc::new(AtomicU32::new(0)),
                Some(frames as usize),
                Some(scale),
            )?;
            archive
                .start_file(
                    &media_name,
                    options.large_file(
                        fs::metadata(&media_path).map_err(|e| e.to_string())?.len()
                            >= u32::MAX as u64,
                    ),
                )
                .map_err(|e| e.to_string())?;
            std::io::copy(
                &mut File::open(&media_path).map_err(|e| e.to_string())?,
                &mut archive,
            )
            .map_err(|e| e.to_string())?;
            fs::remove_file(&media_path).map_err(|e| e.to_string())?;
            children.push(json!({
                "OTIO_SCHEMA":"Clip.1", "name":clip.name, "enabled":!track.muted,
                "metadata":{"DOTAPE":{"source_offset_seconds":clip.offset,"gain":clip.gain,"fade_in_seconds":clip.fade_in,"fade_out_seconds":clip.fade_out,"audio_processing":"baked"}},
                "source_range":range(0,frames,rate), "effects":[],"markers":[],
                "media_reference":{"OTIO_SCHEMA":"ExternalReference.1", "name":clip.name,"metadata":{},"available_range":range(0,frames,rate),"target_url":media_name}
            }));
            cursor = end;
            rendered += 1;
            progress.store(
                10 + (rendered * 85 / prepared.clips.len()) as u32,
                Ordering::Relaxed,
            );
        }
        if cursor < total {
            children.push(gap(total - cursor, rate));
        }
        let name = if track.name.is_empty() {
            format!("트랙 {}", track_index + 1)
        } else {
            track.name.clone()
        };
        tracks.push(json!({"OTIO_SCHEMA":"Track.1","name":name,"kind":"Audio","enabled":!track.muted,"metadata":{"DOTAPE":{"muted":track.muted,"gain":track.gain}},"source_range":null,"children":children,"effects":[],"markers":[]}));
    }
    let timeline = json!({
        "OTIO_SCHEMA":"Timeline.1","name":project.name,"global_start_time":time(0,60),
        "metadata":{"DOTAPE":{"version":env!("CARGO_PKG_VERSION"),"audio_rate":rate,"audio_channels":settings.channels,"master_db":project.master_db,"normalization_scale":scale,"gain_and_fades_baked":true}},
        "tracks":{"OTIO_SCHEMA":"Stack.1","name":"tracks","source_range":null,"metadata":{},"children":tracks,"effects":[],"markers":[]}
    });
    archive
        .start_file("content.otio", options)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(&mut archive, &timeline).map_err(|e| e.to_string())?;
    let file = archive.finish().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&bundle_path, path).map_err(|e| format!("완성 파일을 저장하지 못했습니다: {e}"))?;
    progress.store(100, Ordering::Relaxed);
    Ok(ExportResult {
        path: path.to_path_buf(),
        clipped: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    #[test]
    fn bundle_carries_clips_gaps_mutes_and_the_same_audio_without_external_files() {
        let mut project = export::demo_project();
        project.tracks[0].name = "목소리 A".into();
        project.tracks[1].muted = true;
        project.tracks[2].gain = 0.7;
        project.master_db = -2.0;
        project.clips[0].gain = 0.6;
        project.clips[0].start = 0.213_17;
        project.clips[0].offset = 0.173_13;
        project.clips[0].len = 2.017_19;
        // A cut piece keeps its original envelope and source offset.
        let mut cut = project.clips[0].clone();
        cut.id = 70;
        cut.start = 3.176_31;
        cut.offset += 2.25;
        cut.envelope_offset += 2.25;
        cut.len = 1.731_13;
        project.clips.push(cut);
        let root = std::env::temp_dir().join(format!("dotape-bundle-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("트랙.otioz");
        let settings = ExportSettings {
            split_tracks: true,
            ..Default::default()
        };
        let progress = Arc::new(AtomicU32::new(0));
        export::export(&project, &settings, &path, progress.clone()).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let mut text = String::new();
        zip.by_name("content.otio")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        let timeline: Value = serde_json::from_str(&text).unwrap();
        let tracks = timeline["tracks"]["children"].as_array().unwrap();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0]["name"], "목소리 A");
        assert_eq!(tracks[1]["enabled"], false);
        let total = (project.duration() * 48000.0).ceil() as usize;
        let mut mixed = vec![[0.0_f32; 2]; total];
        let mut clip_count = 0;
        for track in tracks {
            let enabled = track["enabled"].as_bool().unwrap();
            let mut at = 0;
            for item in track["children"].as_array().unwrap() {
                let frames = item["source_range"]["duration"]["value"].as_u64().unwrap() as usize;
                if item["OTIO_SCHEMA"] == "Clip.1" {
                    clip_count += 1;
                    let name = item["media_reference"]["target_url"].as_str().unwrap();
                    assert!(name.starts_with("media/"));
                    let mut media = Vec::new();
                    zip.by_name(name).unwrap().read_to_end(&mut media).unwrap();
                    let mut reader = hound::WavReader::new(Cursor::new(media)).unwrap();
                    assert_eq!(reader.spec().bits_per_sample, 32);
                    assert_eq!(reader.duration() as usize, frames);
                    let samples: Vec<_> = reader.samples::<f32>().map(Result::unwrap).collect();
                    if enabled {
                        for (i, pair) in samples.chunks_exact(2).enumerate() {
                            mixed[at + i][0] += pair[0];
                            mixed[at + i][1] += pair[1];
                        }
                    }
                }
                at += frames;
            }
            assert_eq!(at, total);
        }
        assert_eq!(clip_count, project.clips.len());
        for (frame, actual) in mixed.iter().enumerate() {
            let expected = project.mix_frame(frame as f64 / 48000.0);
            for c in 0..2 {
                assert!(
                    (actual[c] - expected[c]).abs() < 2e-5,
                    "frame {frame}: {actual:?} / {expected:?}"
                );
            }
        }
        assert_eq!(progress.load(Ordering::Relaxed), 100);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
