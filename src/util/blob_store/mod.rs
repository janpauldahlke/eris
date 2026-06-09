//! Content-addressed blob persistence under vault upload dirs (`99_USER_UPLOADED/…`).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::fs;

use crate::executive::error::{FcpError, Result};

/// Result of writing or resolving a content-addressed blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAddressedBlob {
    pub content_hash: String,
    pub relative_path: String,
    /// Unix seconds — original landing time (preserved on dedup hit).
    pub uploaded_at: u64,
    pub dedup_hit: bool,
}

/// SHA-256 hex digest of `bytes` (lowercase).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub async fn uploaded_at_from_metadata(path: &Path) -> u64 {
    fs::metadata(path)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or_else(unix_now_secs)
}

/// Write normalized bytes as `{upload_subdir}/{sha256}.{extension}` or return existing file metadata.
pub async fn persist_content_addressed(
    workspace_root: &Path,
    upload_subdir: &str,
    bytes: &[u8],
    extension: &str,
) -> Result<ContentAddressedBlob> {
    let ext = extension.trim_start_matches('.');
    if ext.is_empty() {
        return Err(FcpError::Config("blob extension must be non-empty".into()));
    }
    let content_hash = sha256_hex(bytes);
    let filename = format!("{content_hash}.{ext}");
    let rel_path = format!(
        "{}/{}",
        upload_subdir.trim_end_matches('/'),
        filename
    );
    let abs_path = workspace_root.join(&rel_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
    }
    let dedup_hit = abs_path.is_file();
    if !dedup_hit {
        fs::write(&abs_path, bytes).await.map_err(FcpError::Io)?;
    }
    let uploaded_at = uploaded_at_from_metadata(&abs_path).await;
    Ok(ContentAddressedBlob {
        content_hash,
        relative_path: rel_path,
        uploaded_at,
        dedup_hit,
    })
}

/// Hash on-disk file bytes (for legacy UUID-named uploads at catalog time).
pub async fn sha256_hex_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).await.map_err(FcpError::Io)?;
    Ok(sha256_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sha256_hex_stable() {
        let h1 = sha256_hex(b"hello");
        let h2 = sha256_hex(b"hello");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn persist_content_addressed_dedups() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let bytes = b"normalized-jpeg-bytes";
        let a = persist_content_addressed(root, "99_USER_UPLOADED/images", bytes, "jpg")
            .await
            .expect("first");
        assert!(!a.dedup_hit);
        let b = persist_content_addressed(root, "99_USER_UPLOADED/images", bytes, "jpg")
            .await
            .expect("second");
        assert!(b.dedup_hit);
        assert_eq!(a.relative_path, b.relative_path);
        assert_eq!(a.content_hash, b.content_hash);
    }
}
