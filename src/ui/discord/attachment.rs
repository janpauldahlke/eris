//! Download Discord CDN attachments and persist through the shared vision normalize pipeline.

use std::path::Path;
use std::time::Duration;

use reqwest::Client;
use serenity::model::prelude::Attachment;

use crate::config::{AppConfig, VisionConfig};
use crate::executive::error::{FcpError, Result};
use crate::presentation::ImageAttachment;
use crate::util::vision::{normalize_upload, persist_normalized_image};

fn extension_allowed(filename: &str, vision: &VisionConfig) -> bool {
    let lower = filename.to_ascii_lowercase();
    vision.allowed_extensions.iter().any(|ext| {
        let e = ext.trim_start_matches('.').to_ascii_lowercase();
        lower.ends_with(&format!(".{e}"))
    })
}

/// True when Serenity reports an image MIME type or the filename matches vision allowlist.
pub(crate) fn is_image_attachment(att: &Attachment, vision: &VisionConfig) -> bool {
    if att
        .content_type
        .as_deref()
        .is_some_and(|ct| ct.starts_with("image/"))
    {
        return true;
    }
    extension_allowed(&att.filename, vision)
}

/// Download the first image attachment, normalize, and save under the vault upload dir.
pub async fn ingest_first_discord_image(
    workspace_root: &Path,
    config: &AppConfig,
    attachments: &[Attachment],
) -> Result<Option<ImageAttachment>> {
    if !config.vision.enabled {
        return Ok(None);
    }
    let vision = &config.vision;
    let image_attachments: Vec<&Attachment> = attachments
        .iter()
        .filter(|a| is_image_attachment(a, vision))
        .collect();
    if image_attachments.is_empty() {
        return Ok(None);
    }
    if image_attachments.len() > 1 {
        tracing::warn!(
            target: "fcp.vision",
            count = image_attachments.len(),
            "Discord message has multiple images; using first only"
        );
    }
    let att = image_attachments[0];

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| FcpError::NetworkFault(format!("Discord image HTTP client: {e}")))?;

    let response = client
        .get(&att.url)
        .send()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("Discord image download failed: {e}")))?;

    if !response.status().is_success() {
        return Err(FcpError::NetworkFault(format!(
            "Discord image download HTTP {}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("Discord image body read failed: {e}")))?;

    if bytes.len() as u64 > vision.max_upload_bytes {
        return Err(FcpError::ToolFault {
            tool_name: "vision:discord".into(),
            reason: format!(
                "Discord attachment {} bytes exceeds max_upload_bytes {}",
                bytes.len(),
                vision.max_upload_bytes
            ),
        });
    }

    let normalized = normalize_upload(&bytes, vision).await?;
    let out_bytes = normalized.bytes.len();
    let out_w = normalized.width;
    let out_h = normalized.height;
    let image = persist_normalized_image(workspace_root, vision, normalized).await?;
    tracing::info!(
        target: "fcp.vision",
        path = %image.relative_path,
        bytes = out_bytes,
        width = out_w,
        height = out_h,
        "Discord image ingested and normalized"
    );
    Ok(Some(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_allowed_matches_jpg_and_png() {
        let vision = VisionConfig::default();
        assert!(extension_allowed("photo.JPG", &vision));
        assert!(extension_allowed("dir/shot.png", &vision));
        assert!(!extension_allowed("notes.txt", &vision));
    }
}
