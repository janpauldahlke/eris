use image::GenericImageView;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::tools::vision::validate::validate_vision_relative_path;

#[derive(Deserialize, JsonSchema)]
pub struct VisionDisplayArgs {
    pub relative_path: String,
}

pub struct VisionDisplayTool {
    pub config: Arc<AppConfig>,
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for VisionDisplayTool {
    fn name(&self) -> &'static str {
        "vision:display"
    }

    fn description(&self) -> &'static str {
        "Show a normalized vault image inline in the operator web UI. Use when the user asks to see/display a known upload path."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VisionDisplayArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Default
    }

    async fn execute(&self, args: Value) -> Result<String> {
        if !self.config.vision.enabled {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "vision is disabled in config".into(),
            });
        }
        let parsed: VisionDisplayArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let rel = parsed.relative_path.replace('\\', "/");
        validate_vision_relative_path(
            &self.workspace_root,
            &self.config.vision.upload_dir,
            &rel,
        )?;
        let filename = rel
            .rsplit('/')
            .next()
            .ok_or_else(|| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "invalid relative_path".into(),
            })?;
        let (width, height) = read_jpeg_dimensions(
            self.workspace_root.join(&rel),
        )
        .await
        .unwrap_or((0, 0));

        Ok(json!({
            "relative_path": rel,
            "preview_url": format!("/api/vision/preview/{filename}"),
            "width": width,
            "height": height,
            "display": true,
        })
        .to_string())
    }
}

async fn read_jpeg_dimensions(path: PathBuf) -> Result<(u32, u32)> {
    tokio::task::spawn_blocking(move || {
        let raw = std::fs::read(&path).map_err(FcpError::Io)?;
        let img = image::load_from_memory(&raw).map_err(|e| FcpError::ToolFault {
            tool_name: "vision:display".into(),
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
            let img: RgbImage = ImageBuffer::from_pixel(5, 2, Rgb([1, 2, 3]));
            img.save(path_buf).expect("jpeg");
        })
        .await
        .expect("join");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn display_returns_preview_url_and_dimensions() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/images/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.jpg";
        write_test_jpeg(root, rel).await;

        let tool = VisionDisplayTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let out = tool
            .execute(json!({ "relative_path": rel }))
            .await
            .expect("display");

        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v.get("display").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(
            v.get("preview_url").and_then(|x| x.as_str()),
            Some(
                "/api/vision/preview/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.jpg"
            )
        );
        assert_eq!(v.get("width").and_then(|x| x.as_u64()), Some(5));
        assert_eq!(v.get("height").and_then(|x| x.as_u64()), Some(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn display_rejects_when_vision_disabled() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let rel = "99_USER_UPLOADED/images/off.jpg";
        write_test_jpeg(root, rel).await;

        let tool = VisionDisplayTool {
            config: Arc::new(AppConfig::default()),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({ "relative_path": rel }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disabled"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn display_rejects_missing_file() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        tokio::fs::create_dir_all(root.join("99_USER_UPLOADED/images"))
            .await
            .expect("mkdir");
        let rel = "99_USER_UPLOADED/images/missing.jpg";

        let tool = VisionDisplayTool {
            config: vision_enabled_config(),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({ "relative_path": rel }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("image"));
    }
}
