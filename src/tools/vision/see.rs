use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::fs;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::tools::vision::client::vision_describe;
use crate::tools::vision::validate::validate_vision_relative_path;

#[derive(Deserialize, JsonSchema)]
pub struct VisionSeeArgs {
    /// Vault-relative path to a normalized JPEG under `[vision].upload_dir`.
    pub relative_path: String,
    /// Optional override; defaults to `[vision].default_prompt`.
    pub prompt: Option<String>,
}

pub struct VisionSeeTool {
    pub config: Arc<AppConfig>,
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for VisionSeeTool {
    fn name(&self) -> &'static str {
        "vision:see"
    }

    fn description(&self) -> &'static str {
        "Describe or analyze a normalized image in the vault upload folder using the multimodal chat model. Use when the user attached an image or asks about a file under the configured upload_dir."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VisionSeeArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Full
    }

    async fn execute(&self, args: Value) -> Result<String> {
        if !self.config.vision.enabled {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "vision is disabled in config".into(),
            });
        }
        let parsed: VisionSeeArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let upload_dir = self.config.vision.upload_dir.as_str();
        let _abs = validate_vision_relative_path(
            &self.workspace_root,
            upload_dir,
            &parsed.relative_path,
        )?;
        let meta = fs::metadata(&_abs).await.map_err(FcpError::Io)?;
        let prompt = parsed
            .prompt
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.config.vision.default_prompt.clone());

        let (description, prompt_tokens, completion_tokens) = vision_describe(
            &self.config,
            &parsed.relative_path.replace('\\', "/"),
            &prompt,
        )
        .await?;

        Ok(json!({
            "path": parsed.relative_path,
            "bytes": meta.len(),
            "description": description,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
        })
        .to_string())
    }
}
