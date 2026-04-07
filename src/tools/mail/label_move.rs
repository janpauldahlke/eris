//! Move a message to a user label (folder) or Spam; create the label if missing.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::util::GmailClient;

const SYSTEM_INBOX: &str = "INBOX";
const SYSTEM_SPAM: &str = "SPAM";

#[derive(Deserialize, JsonSchema)]
pub struct MailMoveArgs {
    /// Message id from mail:check or mail:digest.
    pub message_id: String,
    /// Target folder: user label name (e.g. \"ebay\") or \"spam\" for the Spam folder. Creates a new user label if the name does not exist.
    pub target: String,
    /// If true (default), remove INBOX so the thread leaves the main inbox. Set false to only add the label.
    #[serde(default = "default_remove_from_inbox")]
    pub remove_from_inbox: bool,
}

fn default_remove_from_inbox() -> bool {
    true
}

pub struct MailMoveTool {
    pub client: Arc<GmailClient>,
}

#[async_trait]
impl Tool for MailMoveTool {
    fn name(&self) -> &'static str {
        "mail:move"
    }

    fn description(&self) -> &'static str {
        "Move a Gmail message to a label (folder). Target \"spam\" uses the Spam folder. For any other name, adds that label and creates it if missing. By default removes INBOX so mail leaves the primary inbox."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MailMoveArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet { max_chars: 400 }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MailMoveArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let mid = parsed.message_id.trim();
        if mid.is_empty() {
            return Err(FcpError::SchemaViolation(
                "message_id must be non-empty".into(),
            ));
        }
        let target = parsed.target.trim();
        if target.is_empty() {
            return Err(FcpError::SchemaViolation(
                "target must be non-empty".into(),
            ));
        }

        let (add_ids, remove_ids) = if target.eq_ignore_ascii_case("spam") {
            (
                vec![SYSTEM_SPAM.to_string()],
                if parsed.remove_from_inbox {
                    vec![SYSTEM_INBOX.to_string()]
                } else {
                    vec![]
                },
            )
        } else {
            let label_id = resolve_or_create_user_label(&self.client, target).await?;
            let add = vec![label_id];
            let mut remove = vec![];
            if parsed.remove_from_inbox {
                remove.push(SYSTEM_INBOX.to_string());
            }
            (add, remove)
        };

        self.client
            .modify_message(mid, &add_ids, &remove_ids, "mail:move")
            .await?;

        Ok(format!(
            "[mail:move] Updated message {mid}: added labels {:?}, removed labels {:?}.",
            add_ids, remove_ids
        ))
    }
}

async fn resolve_or_create_user_label(client: &GmailClient, name: &str) -> Result<String> {
    let raw = client.list_labels("mail:move").await?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| {
        tracing::warn!(error = %e, "Gmail labels list is not valid JSON");
        FcpError::ToolFault {
            tool_name: "mail:move".into(),
            reason: "unexpected Gmail labels response".into(),
        }
    })?;

    let labels = v.get("labels").and_then(|l| l.as_array());
    if let Some(arr) = labels {
        for lab in arr {
            let lab_name = lab.get("name").and_then(|n| n.as_str());
            let id = lab.get("id").and_then(|i| i.as_str());
            if lab_name.is_some_and(|n| n.eq_ignore_ascii_case(name)) {
                return id.ok_or_else(|| FcpError::ToolFault {
                    tool_name: "mail:move".into(),
                    reason: "label missing id in Gmail response".into(),
                })
                .map(String::from);
            }
        }
    }

    let created = client.create_label(name, "mail:move").await?;
    let cv: Value = serde_json::from_str(&created).map_err(|e| {
        tracing::warn!(error = %e, "Gmail label create response is not valid JSON");
        FcpError::ToolFault {
            tool_name: "mail:move".into(),
            reason: "unexpected Gmail label create response".into(),
        }
    })?;
    cv.get("id")
        .and_then(|x| x.as_str())
        .map(String::from)
        .ok_or_else(|| FcpError::ToolFault {
            tool_name: "mail:move".into(),
            reason: "created label response missing id".into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_move_args_schema_is_valid() {
        let schema = schemars::schema_for!(MailMoveArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }

    #[test]
    fn default_remove_from_inbox_is_true() {
        assert!(default_remove_from_inbox());
    }
}
