use std::path::Path;

use tokio::fs;
use uuid::Uuid;

use crate::config::AudioConfig;
use crate::executive::error::{FcpError, Result};
use crate::presentation::AudioAttachment;

use super::NormalizedAudio;

/// Write a normalized WAV under `[audio].upload_dir` and return attachment metadata.
pub async fn persist_normalized_audio(
    workspace_root: &Path,
    audio: &AudioConfig,
    normalized: NormalizedAudio,
) -> Result<AudioAttachment> {
    let filename = format!("{}.wav", Uuid::new_v4());
    let rel_path = format!(
        "{}/{}",
        audio.upload_dir.trim_end_matches('/'),
        filename
    );
    let abs_path = workspace_root.join(&rel_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
    }
    fs::write(&abs_path, &normalized.bytes)
        .await
        .map_err(FcpError::Io)?;

    Ok(AudioAttachment {
        relative_path: rel_path,
        preview_url: format!("/api/audio/preview/{filename}"),
        duration_secs: normalized.duration_secs,
    })
}
