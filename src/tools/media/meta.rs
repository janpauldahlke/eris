use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::media::{MediaMetaPatch, TagsPatch, UserNotesPatch, apply_meta_patch, card_to_tool_json};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct MediaMetaArgs {
    #[serde(default)]
    pub relative_path: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub user_notes_set: Option<String>,
    #[serde(default)]
    pub user_notes_append: Option<String>,
    #[serde(default)]
    pub tags_set: Option<Vec<String>>,
    #[serde(default)]
    pub tags_add: Option<Vec<String>>,
    #[serde(default)]
    pub tags_remove: Option<Vec<String>>,
}

pub struct MediaMetaTool {
    pub config: Arc<AppConfig>,
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for MediaMetaTool {
    fn name(&self) -> &'static str {
        "media:meta"
    }

    fn description(&self) -> &'static str {
        "Patch an existing 40_MEDIA catalog card (title, description, notes, tags)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MediaMetaArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Default
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _ = &self.config;
        let parsed: MediaMetaArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let user_notes = if parsed.user_notes_set.is_some() || parsed.user_notes_append.is_some() {
            Some(UserNotesPatch {
                set: parsed.user_notes_set,
                append: parsed.user_notes_append,
            })
        } else {
            None
        };
        let tags = if parsed.tags_set.is_some()
            || parsed.tags_add.is_some()
            || parsed.tags_remove.is_some()
        {
            Some(TagsPatch {
                set: parsed.tags_set,
                add: parsed.tags_add,
                remove: parsed.tags_remove,
            })
        } else {
            None
        };

        let card = apply_meta_patch(
            &self.workspace_root,
            MediaMetaPatch {
                relative_path: parsed.relative_path,
                content_hash: parsed.content_hash,
                title: parsed.title,
                description: parsed.description,
                user_notes,
                tags,
                type_fields: None,
            },
        )
        .await?;

        Ok(serde_json::to_string(&card_to_tool_json(&card)).map_err(FcpError::ParseFault)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{CatalogInput, MediaType, upsert_catalog};
    use crate::util::blob_store::sha256_hex;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::default())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn meta_patches_existing_card() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let bytes = b"meta-patch-bytes";
        let hash = sha256_hex(bytes);
        let rel = format!("99_USER_UPLOADED/images/{hash}.jpg");
        tokio::fs::create_dir_all(root.join("99_USER_UPLOADED/images"))
            .await
            .expect("mkdir");
        tokio::fs::write(root.join(&rel), bytes)
            .await
            .expect("write");

        upsert_catalog(
            root,
            CatalogInput {
                relative_path: rel.clone(),
                title: "Before".into(),
                media_type: Some(MediaType::Image),
                tags: vec!["old".into()],
                description: "desc".into(),
                user_notes: String::new(),
                uploaded_at: Some(100),
                type_fields: Default::default(),
            },
        )
        .await
        .expect("seed card");

        let tool = MediaMetaTool {
            config: test_config(),
            workspace_root: root.to_path_buf(),
        };
        let out = tool
            .execute(json!({
                "content_hash": hash,
                "title": "After",
                "tags_add": ["new"],
            }))
            .await
            .expect("meta");

        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v.get("title").and_then(|x| x.as_str()), Some("After"));
        let card_path = root
            .join("40_MEDIA")
            .join(&hash)
            .join("media.json");
        let on_disk: crate::media::MediaCard =
            serde_json::from_str(&tokio::fs::read_to_string(&card_path).await.expect("read"))
                .expect("card");
        assert!(on_disk.tags.iter().any(|t| t == "old"));
        assert!(on_disk.tags.iter().any(|t| t == "new"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn meta_errors_when_card_missing() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let tool = MediaMetaTool {
            config: test_config(),
            workspace_root: root.to_path_buf(),
        };
        let err = tool
            .execute(json!({
                "content_hash": "f".repeat(64),
                "title": "Ghost",
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no catalog card"));
    }
}
