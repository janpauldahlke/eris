//! Trash or permanently delete a message.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::util::GmailClient;

#[derive(Deserialize, JsonSchema)]
pub struct MailDeleteArgs {
    /// Message id from mail:check or mail:digest.
    pub message_id: String,
    /// If true, permanently delete (not recoverable from Trash). Default false = move to Trash.
    #[serde(default)]
    pub permanent: Option<bool>,
}

pub struct MailDeleteTool {
    pub client: Arc<GmailClient>,
}

#[async_trait]
impl Tool for MailDeleteTool {
    fn name(&self) -> &'static str {
        "mail:delete"
    }

    fn description(&self) -> &'static str {
        "Delete a Gmail message: by default moves to Trash (recoverable). Set permanent=true only when the user explicitly wants permanent deletion."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MailDeleteArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet { max_chars: 400 }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MailDeleteArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let id = parsed.message_id.trim();
        if id.is_empty() {
            return Err(FcpError::SchemaViolation(
                "message_id must be non-empty".into(),
            ));
        }

        if parsed.permanent.unwrap_or(false) {
            self.client
                .delete_message_permanent(id, "mail:delete")
                .await?;
            return Ok(format!(
                "[mail:delete] Message permanently deleted (id={id})."
            ));
        }

        let raw = self.client.trash_message(id, "mail:delete").await?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "Gmail trash response is not valid JSON");
            FcpError::ToolFault {
                tool_name: "mail:delete".into(),
                reason: "unexpected Gmail API response (invalid JSON)".into(),
            }
        })?;
        let new_id = v.get("id").and_then(|x| x.as_str()).unwrap_or(id);
        Ok(format!(
            "[mail:delete] Message moved to Trash (id={new_id})."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_delete_args_schema_is_valid() {
        let schema = schemars::schema_for!(MailDeleteArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }
}
