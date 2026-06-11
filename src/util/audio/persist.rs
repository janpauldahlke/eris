use std::path::Path;

use crate::config::AudioConfig;
use crate::executive::error::Result;
use crate::presentation::AudioAttachment;
use crate::util::blob_store::persist_content_addressed;

use super::NormalizedAudio;

/// Write a normalized WAV under `[audio].upload_dir` and return attachment metadata.
pub async fn persist_normalized_audio(
    workspace_root: &Path,
    audio: &AudioConfig,
    normalized: NormalizedAudio,
) -> Result<AudioAttachment> {
    let blob = persist_content_addressed(
        workspace_root,
        &audio.upload_dir,
        &normalized.bytes,
        "wav",
    )
    .await?;
    let filename = blob
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or("audio.wav")
        .to_string();
    Ok(AudioAttachment {
        relative_path: blob.relative_path,
        preview_url: format!("/api/audio/preview/{filename}"),
        duration_secs: normalized.duration_secs,
    })
}
