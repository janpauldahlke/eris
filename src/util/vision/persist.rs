use std::path::Path;

use tokio::fs;
use uuid::Uuid;

use crate::config::VisionConfig;
use crate::executive::error::{FcpError, Result};
use crate::presentation::ImageAttachment;

use super::NormalizedImage;

/// Write a normalized JPEG under `[vision].upload_dir` and return attachment metadata.
pub async fn persist_normalized_image(
    workspace_root: &Path,
    vision: &VisionConfig,
    normalized: NormalizedImage,
) -> Result<ImageAttachment> {
    let filename = format!("{}.jpg", Uuid::new_v4());
    let rel_path = format!(
        "{}/{}",
        vision.upload_dir.trim_end_matches('/'),
        filename
    );
    let abs_path = workspace_root.join(&rel_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
    }
    fs::write(&abs_path, &normalized.bytes)
        .await
        .map_err(FcpError::Io)?;

    Ok(ImageAttachment {
        relative_path: rel_path,
        preview_url: format!("/api/vision/preview/{filename}"),
        width: normalized.width,
        height: normalized.height,
    })
}
