use image::GenericImageView;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::media::{
    CatalogInput, MediaType, card_to_tool_json, infer_media_type_from_path, upsert_catalog,
};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::tools::vision::validate::validate_vision_relative_path;
use crate::util::blob_store::uploaded_at_from_metadata;

#[derive(Deserialize, JsonSchema)]
pub struct MediaCatalogArgs {
    pub relative_path: String,
    /// Short label for recall. Optional when `description` is set (derived from first sentence).
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub user_notes: String,
}

pub struct MediaCatalogTool {
    pub config: Arc<AppConfig>,
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for MediaCatalogTool {
    fn name(&self) -> &'static str {
        "media:catalog"
    }

    fn description(&self) -> &'static str {
        "Create or update a 40_MEDIA catalog card for a user-uploaded blob (v1: images). Title is optional when description is present (e.g. after vision:see)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MediaCatalogArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Default
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MediaCatalogArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let rel = parsed.relative_path.replace('\\', "/");
        let media_type = if let Some(ref mt) = parsed.media_type {
            parse_media_type_str(mt)?
        } else {
            infer_media_type_from_path(&rel).ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "could not infer media_type; pass media_type explicitly".into(),
            })?
        };

        if media_type != MediaType::Image {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "v1 supports media:catalog for images only".into(),
            });
        }

        if !self.config.vision.enabled {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "image catalog requires [vision] enabled".into(),
            });
        }

        let abs = validate_vision_relative_path(
            &self.workspace_root,
            &self.config.vision.upload_dir,
            &rel,
        )?;
        let uploaded_at = uploaded_at_from_metadata(&abs).await;
        let mut type_fields = std::collections::BTreeMap::new();
        if let Ok(dims) = read_jpeg_dimensions(abs).await {
            type_fields.insert("width".into(), json!(dims.0));
            type_fields.insert("height".into(), json!(dims.1));
        }

        let title = resolve_catalog_title(parsed.title.as_deref(), &parsed.description)?;

        let card = upsert_catalog(
            &self.workspace_root,
            CatalogInput {
                relative_path: rel,
                title,
                media_type: Some(media_type),
                tags: parsed.tags,
                description: parsed.description,
                user_notes: parsed.user_notes,
                uploaded_at: Some(uploaded_at),
                type_fields,
            },
        )
        .await?;

        Ok(serde_json::to_string(&card_to_tool_json(&card)).map_err(FcpError::ParseFault)?)
    }
}

/// Use explicit title when provided; otherwise derive a short label from `vision:see` description.
fn resolve_catalog_title(title: Option<&str>, description: &str) -> Result<String> {
    if let Some(explicit) = title.map(str::trim).filter(|t| !t.is_empty()) {
        return Ok(explicit.to_string());
    }
    derive_title_from_description(description).ok_or_else(|| FcpError::ToolFault {
        tool_name: "media:catalog".into(),
        reason: "no title — run vision:see on the image first and pass description, or supply a short title"
            .into(),
    })
}

fn derive_title_from_description(description: &str) -> Option<String> {
    let trimmed = description.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first = trimmed
        .split(['\n', '.', '!', '?'])
        .next()
        .unwrap_or(trimmed)
        .trim();
    if first.is_empty() {
        return None;
    }
    const MAX: usize = 80;
    if first.chars().count() <= MAX {
        Some(first.to_string())
    } else {
        Some(format!("{}...", first.chars().take(MAX.saturating_sub(3)).collect::<String>()))
    }
}

fn parse_media_type_str(raw: &str) -> Result<MediaType> {
    match raw.trim().to_lowercase().as_str() {
        "image" => Ok(MediaType::Image),
        "audio" => Ok(MediaType::Audio),
        "document" => Ok(MediaType::Document),
        other => Err(FcpError::ToolFault {
            tool_name: "media:catalog".into(),
            reason: format!("unknown media_type `{other}`"),
        }),
    }
}

async fn read_jpeg_dimensions(path: PathBuf) -> Result<(u32, u32)> {
    let path = path;
    tokio::task::spawn_blocking(move || {
        let raw = std::fs::read(&path).map_err(FcpError::Io)?;
        let img = image::load_from_memory(&raw).map_err(|e| FcpError::ToolFault {
            tool_name: "media:catalog".into(),
            reason: format!("read dimensions: {e}"),
        })?;
        Ok(img.dimensions())
    })
    .await
    .map_err(|e| FcpError::Config(format!("dimension join: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::MediaCard;
    use crate::util::blob_store::sha256_hex_file;
    use std::path::Path;
    use tempfile::tempdir;

    fn vision_enabled_config() -> Arc<AppConfig> {
        let mut config = AppConfig::default();
        config.vision.enabled = true;
        config.vision.upload_dir = "99_USER_UPLOADED/images".into();
        Arc::new(config)
    }

    async fn write_test_jpeg(root: &Path, rel: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.expect("mkdir");
        }
        let path_buf = path.clone();
        tokio::task::spawn_blocking(move || {
            use image::{ImageBuffer, Rgb, RgbImage};
            let img: RgbImage = ImageBuffer::from_pixel(4, 3, Rgb([10, 20, 30]));
            img.save(path_buf).expect("jpeg");
        })
        .await
        .expect("join");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_creates_card_for_image() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/images/catalog-test.jpg";
        write_test_jpeg(root, rel).await;
        let hash = sha256_hex_file(&root.join(rel))
            .await
            .expect("hash");

        let tool = MediaCatalogTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let out = tool
            .execute(json!({
                "relative_path": rel,
                "title": "Fish truck",
                "tags": ["food"],
                "description": "A truck with fish",
            }))
            .await
            .expect("catalog");

        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v.get("title").and_then(|x| x.as_str()), Some("Fish truck"));
        assert_eq!(v.get("content_hash").and_then(|x| x.as_str()), Some(hash.as_str()));
        let card_path = root.join("40_MEDIA").join(&hash).join("media.json");
        assert!(card_path.is_file());
        let on_disk: MediaCard =
            serde_json::from_str(&tokio::fs::read_to_string(&card_path).await.expect("read"))
                .expect("card json");
        assert_eq!(on_disk.type_fields.get("width").and_then(|x| x.as_u64()), Some(4));
    }

    #[test]
    fn derive_title_from_description_uses_first_sentence() {
        assert_eq!(
            derive_title_from_description("Artisan fish truck at the market. Busy lunch crowd."),
            Some("Artisan fish truck at the market".into())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_derives_title_when_omitted() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/images/auto-title.jpg";
        write_test_jpeg(root, rel).await;

        let tool = MediaCatalogTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let out = tool
            .execute(json!({
                "relative_path": rel,
                "description": "Red food truck with fish logo at Zen street market",
            }))
            .await
            .expect("catalog");

        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(
            v.get("title").and_then(|x| x.as_str()),
            Some("Red food truck with fish logo at Zen street market")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_rejects_empty_title_and_description() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/images/bare.jpg";
        write_test_jpeg(root, rel).await;

        let tool = MediaCatalogTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({ "relative_path": rel }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("vision:see"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_rejects_when_vision_disabled() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/images/disabled.jpg";
        write_test_jpeg(root, rel).await;

        let tool = MediaCatalogTool {
            config: Arc::new(AppConfig::default()),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({
                "relative_path": rel,
                "title": "Nope",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("vision"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_rejects_non_image_media_type() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/audio/note.wav";
        tokio::fs::create_dir_all(root.join("99_USER_UPLOADED/audio"))
            .await
            .expect("mkdir");
        tokio::fs::write(root.join(rel), b"wav")
            .await
            .expect("write");

        let tool = MediaCatalogTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({
                "relative_path": rel,
                "title": "Audio",
                "media_type": "audio",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("images only"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn catalog_rejects_path_outside_upload_dir() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        tokio::fs::create_dir_all(root.join("00_Invariants"))
            .await
            .expect("mkdir");
        let rel = "00_Invariants/secret.jpg";
        write_test_jpeg(root, rel).await;

        let tool = MediaCatalogTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({
                "relative_path": rel,
                "title": "Sneak",
                "media_type": "image",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("upload dir"));
    }
}
