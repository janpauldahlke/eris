use std::path::Path;

use crate::config::VisionConfig;
use crate::executive::error::Result;
use crate::presentation::ImageAttachment;
use crate::util::blob_store::persist_content_addressed;

use super::NormalizedImage;

/// Write a normalized JPEG under `[vision].upload_dir` and return attachment metadata.
pub async fn persist_normalized_image(
    workspace_root: &Path,
    vision: &VisionConfig,
    normalized: NormalizedImage,
) -> Result<ImageAttachment> {
    let blob = persist_content_addressed(
        workspace_root,
        &vision.upload_dir,
        &normalized.bytes,
        "jpg",
    )
    .await?;
    let filename = blob
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or("image.jpg")
        .to_string();
    Ok(ImageAttachment {
        relative_path: blob.relative_path,
        preview_url: format!("/api/vision/preview/{filename}"),
        width: normalized.width,
        height: normalized.height,
    })
}
