use std::path::{Component, Path, PathBuf};

use crate::executive::error::{FcpError, Result};

/// Ensure `relative_path` stays under `workspace_root` and within `upload_dir`.
pub fn validate_audio_relative_path(
    workspace_root: &Path,
    upload_dir: &str,
    relative_path: &str,
) -> Result<PathBuf> {
    let upload_norm = upload_dir.replace('\\', "/").trim_matches('/').to_string();
    let rel_norm = relative_path.replace('\\', "/").trim_start_matches('/').to_string();
    if rel_norm.contains("..") {
        return Err(FcpError::ToolFault {
            tool_name: "audio:ingress".into(),
            reason: "path traversal denied".into(),
        });
    }
    let prefix = format!("{upload_norm}/");
    if !rel_norm.starts_with(&prefix) {
        return Err(FcpError::ToolFault {
            tool_name: "audio:ingress".into(),
            reason: format!("path must be under vault upload dir `{upload_dir}`"),
        });
    }

    let target = workspace_root.join(&rel_norm);
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let canonical_target = target
        .canonicalize()
        .map_err(|e| FcpError::ToolFault {
            tool_name: "audio:ingress".into(),
            reason: format!("audio not found: {e}"),
        })?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(FcpError::ToolFault {
            tool_name: "audio:ingress".into(),
            reason: "path traversal denied".into(),
        });
    }

    let ext = canonical_target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !ext.eq_ignore_ascii_case("wav") {
        return Err(FcpError::ToolFault {
            tool_name: "audio:ingress".into(),
            reason: "voice uploads must be normalized WAV under upload_dir".into(),
        });
    }

    Ok(canonical_target)
}

/// Filename allowlist for preview route (`{uuid}.wav`).
pub fn preview_filename_allowed(name: &str) -> bool {
    let path = Path::new(name);
    if path.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return false;
    }
    let file_name = match path.file_name().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return false,
    };
    let lower = file_name.to_ascii_lowercase();
    if !lower.ends_with(".wav") {
        return false;
    }
    let stem = lower.trim_end_matches(".wav");
    !stem.is_empty()
        && stem
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-')
        && stem.len() >= 32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn preview_filename_allows_uuid_wav() {
        assert!(preview_filename_allowed(
            "550e8400-e29b-41d4-a716-446655440000.wav"
        ));
        assert!(!preview_filename_allowed("../secret.wav"));
        assert!(!preview_filename_allowed("not-uuid.wav"));
    }

    #[tokio::test]
    async fn validate_rejects_traversal() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("99_USER_UPLOADED/audio")).expect("mkdir");
        let err = validate_audio_relative_path(
            root,
            "99_USER_UPLOADED/audio",
            "../../etc/passwd",
        )
        .unwrap_err();
        assert!(err.to_string().contains("traversal") || err.to_string().contains("upload"));
    }
}
