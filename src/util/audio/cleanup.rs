use std::path::Path;

use crate::config::AudioConfig;
use crate::executive::error::{FcpError, Result};

/// Remove normalized WAV files under `[audio].upload_dir` (chat exit housekeeping).
pub fn purge_upload_dir(workspace_root: &Path, audio: &AudioConfig) -> Result<usize> {
    let dir = workspace_root.join(audio.upload_dir.trim_end_matches('/'));
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in std::fs::read_dir(&dir).map_err(FcpError::Io)? {
        let entry = entry.map_err(FcpError::Io)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_wav = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
        if !is_wav {
            continue;
        }
        std::fs::remove_file(&path).map_err(FcpError::Io)?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn purge_removes_wav_only() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        let upload = root.join("99_USER_UPLOADED/audio");
        std::fs::create_dir_all(&upload).expect("mkdir");
        std::fs::write(upload.join("a.wav"), b"wav").expect("write");
        std::fs::write(upload.join("keep.txt"), b"x").expect("write");

        let config = AudioConfig {
            upload_dir: "99_USER_UPLOADED/audio".into(),
            ..AudioConfig::default()
        };
        let n = purge_upload_dir(root, &config).expect("purge");
        assert_eq!(n, 1);
        assert!(!upload.join("a.wav").exists());
        assert!(upload.join("keep.txt").exists());
    }
}
