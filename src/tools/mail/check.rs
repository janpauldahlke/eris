use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::generated::gws_types::gmail::ListMessagesResponse;
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::mail::common::format_metadata_line;
use crate::tools::traits::Tool;
use crate::util::GmailClient;

#[derive(Deserialize, JsonSchema)]
pub struct MailCheckArgs {
    /// Gmail search query (e.g. "is:unread", "from:boss@co.com"). Omit for recent messages.
    #[serde(default)]
    pub query: Option<String>,
    /// Maximum number of messages to return (default 10, max 50).
    #[serde(default)]
    pub max_results: Option<u32>,
}

pub struct MailCheckTool {
    pub client: Arc<GmailClient>,
}

#[async_trait]
impl Tool for MailCheckTool {
    fn name(&self) -> &'static str {
        "mail:check"
    }

    fn description(&self) -> &'static str {
        "Check Gmail inbox: list recent or filtered messages. Each line includes message id, thread id, subject, from, date, and a short preview (from Gmail metadata). Use mail:read with a message id for full body text."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MailCheckArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MailCheckArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let max = parsed.max_results.unwrap_or(10).min(50);
        let query = parsed.query.as_deref();

        let raw = self.client.list_messages(query, max).await?;
        let list: ListMessagesResponse = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "failed to parse Gmail list response");
            FcpError::ToolFault {
                tool_name: "mail:check".into(),
                reason: "unexpected Gmail API response format".into(),
            }
        })?;

        let messages = list.messages.as_deref().unwrap_or(&[]);
        if messages.is_empty() {
            return Ok(format!(
                "[mail:check] No messages found{}.",
                query
                    .map(|q| format!(" for query \"{q}\""))
                    .unwrap_or_default()
            ));
        }

        let count = messages.len();
        let estimate = list.result_size_estimate.unwrap_or(count as u32);
        let mut out = format!(
            "[mail:check] Showing {count} of ~{estimate} messages{} (subject/from/date from metadata):\n\n",
            query
                .map(|q| format!(" matching \"{q}\""))
                .unwrap_or_default()
        );

        for msg in messages {
            let id = msg.id.as_deref().unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let thread = msg.thread_id.as_deref().unwrap_or("?");
            match self.client.get_message_metadata(id).await {
                Ok(meta_raw) => match serde_json::from_str::<Value>(&meta_raw) {
                    Ok(v) => {
                        out.push_str(&format_metadata_line(&v, id, thread));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, message_id = %id, "mail:check metadata JSON parse");
                        out.push_str(&format!(
                            "- id={id} | thread={thread} | subject: (parse error) | use mail:read\n"
                        ));
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, message_id = %id, "mail:check metadata fetch failed");
                    out.push_str(&format!(
                        "- id={id} | thread={thread} | subject: (unavailable) | use mail:read for details\n"
                    ));
                }
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::mail::common::format_metadata_line;
    use serde_json::json;

    #[test]
    fn mail_check_args_schema_is_valid() {
        let schema = schemars::schema_for!(MailCheckArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }

    #[test]
    fn format_metadata_line_includes_subject_and_from() {
        let v = json!({
            "id": "msg1",
            "threadId": "th1",
            "snippet": "Hello there",
            "payload": {
                "headers": [
                    {"name": "Subject", "value": "Meeting tomorrow"},
                    {"name": "From", "value": "alice@example.com"},
                    {"name": "Date", "value": "Mon, 6 Apr 2026 12:00:00 +0000"}
                ]
            }
        });
        let line = format_metadata_line(&v, "msg1", "th1");
        assert!(line.contains("subject: Meeting tomorrow"));
        assert!(line.contains("from: alice@example.com"));
        assert!(line.contains("preview: Hello there"));
        assert!(line.contains("thread=th1"));
    }

    #[test]
    fn parse_list_response_empty() {
        let raw = r#"{"resultSizeEstimate":0}"#;
        let list: ListMessagesResponse = serde_json::from_str(raw).expect("parse");
        assert!(list.messages.is_none());
        assert_eq!(list.result_size_estimate, Some(0));
    }

    #[test]
    fn parse_list_response_with_messages() {
        let raw = r#"{"messages":[{"id":"a","threadId":"t1"},{"id":"b","threadId":"t2"}],"resultSizeEstimate":42}"#;
        let list: ListMessagesResponse = serde_json::from_str(raw).expect("parse");
        let msgs = list.messages.expect("messages");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id.as_deref(), Some("a"));
        assert_eq!(list.result_size_estimate, Some(42));
    }
}
