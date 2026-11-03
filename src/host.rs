//! 다른 프로그램(호스트)이 DOTAPE에 녹음을 맡길 때 쓰는 작은 약속.
//!
//! DOTAPE는 호스트가 누구인지 모른다. 환경 변수 세 개와 `handoff.json` 하나가 전부이고,
//! 아무것도 설정하지 않으면 평소와 똑같이 동작한다.
//!
//! - `SORI_DATA_DIR`   작업·원본 저장 위치(`session.rs`). 설정되어 있을 때만 `handoff.json`을 쓴다.
//! - `SORI_EXPORT_DIR` 내보내기 대화상자가 처음 열리는 폴더. 없으면 만든다.
//! - `SORI_HOST`       호스트 이름(자유 텍스트, 예: `DOTRACK`). 있으면 하단에 안내 한 줄을 보여준다.
//!
//! `handoff.json`은 녹음이 끝날 때, 내보내기가 끝날 때, 창을 닫을 때 통째로 다시 쓴다.
//! 호스트는 `last_export`, 없으면 `last_take`를 집어 가면 된다. `closed`가 `true`면 DOTAPE가 정상 종료한 것이다.

use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const HANDOFF_FILE: &str = "handoff.json";

fn env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

/// `SORI_HOST`. 비어 있으면 호스트 없이 실행 중이다.
pub fn host() -> Option<String> {
    let name = std::env::var("SORI_HOST").ok()?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// `SORI_EXPORT_DIR`. 폴더가 없으면 만든다. 만들지 못해도 경로는 돌려준다.
pub fn export_dir() -> Option<PathBuf> {
    let dir = env_path("SORI_EXPORT_DIR")?;
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// `handoff.json`을 둘 곳. `SORI_DATA_DIR`가 없으면 쓰지 않는다.
pub fn handoff_dir() -> Option<PathBuf> {
    env_path("SORI_DATA_DIR")
}

/// 저장 대화상자가 열릴 폴더. 마지막으로 저장한 곳이 있으면 그 폴더, 없으면 호스트가 정한 폴더.
pub fn dialog_dir(last_export: Option<&Path>, host_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(parent) = last_export
        .and_then(Path::parent)
        .filter(|p| !p.as_os_str().is_empty() && p.is_dir())
    {
        return Some(parent.to_path_buf());
    }
    host_dir.filter(|p| p.is_dir()).map(Path::to_path_buf)
}

#[derive(Serialize)]
struct Handoff<'a> {
    version: u32,
    host: Option<&'a str>,
    last_take: Option<String>,
    last_export: Option<String>,
    takes: Vec<String>,
    exports: Vec<String>,
    updated: u64,
    closed: bool,
}

/// `dir/handoff.json`을 임시 파일에 쓴 뒤 바꿔치기한다. 디스크에 남아 있는 항목만 적는다.
/// 내보내기 항목은 파일이거나(합친 파일) 폴더다(트랙별 파일).
pub fn write_handoff(
    dir: &Path,
    host: Option<&str>,
    takes: &[PathBuf],
    exports: &[PathBuf],
    closed: bool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let takes: Vec<String> = takes
        .iter()
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let exports: Vec<String> = exports
        .iter()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let handoff = Handoff {
        version: 1,
        host,
        last_take: takes.last().cloned(),
        last_export: exports.last().cloned(),
        takes,
        exports,
        updated: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        closed,
    };
    let path = dir.join(HANDOFF_FILE);
    let temp = dir.join(format!("{HANDOFF_FILE}.tmp"));
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(&handoff).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        e.to_string()
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::var_os("SORI_TEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("Sori-tests"))
            .join(format!("host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn handoff_lists_only_files_that_exist_and_replaces_atomically() {
        let root = scratch("handoff");
        let data = root.join("data");
        let take = root.join("take-1.wav");
        let export = root.join("mix.wav");
        let stems = root.join("트랙 - mix");
        std::fs::write(&take, b"RIFF").unwrap();
        std::fs::write(&export, b"RIFF").unwrap();
        std::fs::create_dir(&stems).unwrap();
        let path = write_handoff(
            &data,
            Some("DOTRACK"),
            &[take.clone(), root.join("gone.wav")],
            &[root.join("missing.flac"), export.clone(), stems.clone()],
            false,
        )
        .unwrap();
        assert_eq!(path, data.join("handoff.json"));
        assert!(!data.join("handoff.json.tmp").exists());
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["host"], "DOTRACK");
        assert_eq!(v["closed"], false);
        assert_eq!(v["last_take"], take.to_string_lossy().as_ref());
        assert_eq!(v["last_export"], stems.to_string_lossy().as_ref());
        assert_eq!(v["takes"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["exports"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            [
                export.to_string_lossy().into_owned(),
                stems.to_string_lossy().into_owned()
            ]
        );
        assert!(v["updated"].as_u64().unwrap() > 1_700_000_000);

        std::fs::remove_file(&export).unwrap();
        let path = write_handoff(&data, None, &[], &[export], true).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(v["host"].is_null());
        assert!(v["last_take"].is_null());
        assert!(v["last_export"].is_null());
        assert_eq!(v["exports"].as_array().unwrap().len(), 0);
        assert_eq!(v["closed"], true);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dialog_opens_where_the_last_export_went_else_in_the_host_folder() {
        let root = scratch("dialog");
        let host = root.join("export");
        std::fs::create_dir(&host).unwrap();
        assert_eq!(dialog_dir(None, None), None);
        assert_eq!(dialog_dir(None, Some(&host)), Some(host.clone()));
        assert_eq!(dialog_dir(None, Some(&root.join("nowhere"))), None);
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let last = elsewhere.join("mix.wav");
        assert_eq!(
            dialog_dir(Some(&last), Some(&host)),
            Some(elsewhere.clone())
        );
        let stems = elsewhere.join("트랙 - mix");
        assert_eq!(
            dialog_dir(Some(&stems), Some(&host)),
            Some(elsewhere.clone())
        );
        let vanished = root.join("vanished").join("mix.wav");
        assert_eq!(dialog_dir(Some(&vanished), Some(&host)), Some(host.clone()));
        assert_eq!(
            dialog_dir(Some(Path::new("mix.wav")), Some(&host)),
            Some(host)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
