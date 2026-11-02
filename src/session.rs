use crate::{
    audio::RecordSettings,
    export::ExportSettings,
    model::{Clip, Project, Source, Track},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn data_dir() -> PathBuf {
    std::env::var_os("SORI_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("Sori")
        })
}
pub fn next_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}
pub fn recording_path(id: u64) -> Result<PathBuf, String> {
    let path = data_dir().join("recordings");
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path.join(format!("take-{id}.wav")))
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    name: String,
    tracks: Vec<Track>,
    clips: Vec<Clip>,
    sources: HashMap<u64, String>,
    master_db: f32,
    record: RecordSettings,
    export: ExportSettings,
}

pub fn save(
    project: &Project,
    record: &RecordSettings,
    export: &ExportSettings,
) -> Result<(), String> {
    let root = data_dir();
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let saved = Saved {
        version: 1,
        name: project.name.clone(),
        tracks: project.tracks.clone(),
        clips: project.clips.clone(),
        sources: project
            .sources
            .iter()
            .filter(|(id, _)| project.clips.iter().any(|c| c.source == **id))
            .map(|(id, s)| (*id, s.path.clone()))
            .collect(),
        master_db: project.master_db,
        record: record.clone(),
        export: export.clone(),
    };
    let temp = root.join("session.json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec(&saved).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, root.join("session.json")).map_err(|e| e.to_string())
}

pub fn load() -> Result<Option<(Project, RecordSettings, ExportSettings)>, String> {
    let path = data_dir().join("session.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if saved.version != 1 || !saved.master_db.is_finite() {
        return Err("저장된 작업의 형식이 올바르지 않습니다.".into());
    }
    let mut sources = HashMap::new();
    for (id, path) in &saved.sources {
        sources.insert(*id, Arc::new(read_wav(*id, Path::new(path))?));
    }
    for c in &saved.clips {
        let src = sources.get(&c.source).ok_or("녹음 원본이 없습니다.")?;
        if c.track >= saved.tracks.len()
            || [
                c.start,
                c.offset,
                c.len,
                c.fade_in,
                c.fade_out,
                c.envelope_offset,
                c.envelope_len,
            ]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
            || c.len <= 0.0
            || c.offset + c.len > src.duration() + 0.001
            || !c.gain.is_finite()
        {
            return Err("저장된 클립의 위치 정보가 올바르지 않습니다.".into());
        }
    }
    saved.export.validate()?;
    if saved.tracks.iter().any(|t| !t.gain.is_finite()) || !saved.record.gain_db.is_finite() {
        return Err("저장된 볼륨이 올바르지 않습니다.".into());
    }
    Ok(Some((
        Project {
            name: saved.name,
            tracks: saved.tracks,
            clips: saved.clips,
            sources,
            master_db: saved.master_db,
        },
        saved.record,
        saved.export,
    )))
}

pub fn read_wav(id: u64, path: &Path) -> Result<Source, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("녹음 원본을 열지 못했습니다: {e}"))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > 2 || spec.sample_rate == 0 {
        return Err("모노 또는 스테레오 WAV만 지원합니다.".into());
    }
    let samples: Result<Vec<f32>, _> = if spec.sample_format == hound::SampleFormat::Float {
        reader.samples::<f32>().collect()
    } else {
        let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
        reader
            .samples::<i32>()
            .map(|v| v.map(|x| x as f32 / scale))
            .collect()
    };
    let samples = samples.map_err(|e| e.to_string())?;
    if samples.iter().any(|v| !v.is_finite()) {
        return Err("WAV에 올바르지 않은 샘플 값이 있습니다.".into());
    }
    Ok(Source::new(
        id,
        samples,
        spec.sample_rate,
        spec.channels as usize,
        path.to_string_lossy().to_string(),
    ))
}
