//! Batch list of recent mail (default: messages from today, local date) for summarization.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, Local};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::generated::gws_types::gmail::ListMessagesResponse;
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::mail::common::format_metadata_line;
use crate::tools::traits::Tool;
use crate::util::GmailClient;

const DEFAULT_MAX: u32 = 20;
const CAP_MAX: u32 = 50;

#[derive(Deserialize, JsonSchema)]
pub struct MailDigestArgs {
    /// Gmail search query. If omitted, uses `after:YYYY/MM/DD` for **today** in the local timezone (Gmail search semantics).
    #[serde(default)]
    pub query: Option<String>,
    /// Maximum messages to include (default 20, max 50). Uses metadata rows only (no full bodies).
    #[serde(default)]
    pub max_messages: Option<u32>,
}

pub struct MailDigestTool {
    pub client: Arc<GmailClient>,
}

fn effective_query(args: &MailDigestArgs) -> String {
    args.query.clone().unwrap_or_else(|| {
        let d = Local::now().date_naive();
        format!("after:{}/{}/{}", d.year(), d.month(), d.day())
    })
}

#[async_trait]
impl Tool for MailDigestTool {
    fn name(&self) -> &'static str {
        "mail:digest"
    }

    fn description(&self) -> &'static str {
        "List multiple Gmail messages (metadata + snippet) in one block for summarization — default is mail from today (local date). Use when the user asks for a digest or summary of recent mail without reading each body separately."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MailDigestArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MailDigestArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let max = parsed
            .max_messages
            .unwrap_or(DEFAULT_MAX)
            .min(CAP_MAX)
            .max(1);
        let q = effective_query(&parsed);

        let raw = self.client.list_messages(Some(&q), max).await?;
        let list: ListMessagesResponse = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "failed to parse Gmail list response (digest)");
            FcpError::ToolFault {
                tool_name: "mail:digest".into(),
                reason: "unexpected Gmail API response format".into(),
            }
        })?;

        let messages = list.messages.as_deref().unwrap_or(&[]);
        if messages.is_empty() {
            return Ok(format!(
                "[mail:digest] No messages found for query \"{q}\"."
            ));
        }

        let count = messages.len();
        let estimate = list.result_size_estimate.unwrap_or(count as u32);
        let mut out = format!(
            "[mail:digest] Showing {count} of ~{estimate} messages for \"{q}\" (metadata + snippet; use mail:read for full body):\n\n",
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
                        tracing::warn!(error = %e, message_id = %id, "mail:digest metadata JSON parse");
                        out.push_str(&format!(
                            "- id={id} | thread={thread} | subject: (parse error) | use mail:read\n"
                        ));
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, message_id = %id, "mail:digest metadata fetch failed");
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

    #[test]
    fn mail_digest_args_schema_is_valid() {
        let schema = schemars::schema_for!(MailDigestArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }

    #[test]
    fn effective_query_uses_after_when_none() {
        let args = MailDigestArgs {
            query: None,
            max_messages: None,
        };
        let q = effective_query(&args);
        assert!(q.starts_with("after:"));
        assert_eq!(q.matches('/').count(), 2);
    }

    #[test]
    fn effective_query_respects_explicit() {
        let args = MailDigestArgs {
            query: Some("is:unread".into()),
            max_messages: None,
        };
        assert_eq!(effective_query(&args), "is:unread");
    }
}
