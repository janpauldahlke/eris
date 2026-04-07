use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use htmd::HtmlToMarkdown;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::util::GmailClient;

const MAX_BODY_CHARS: usize = 8000;

#[derive(Deserialize, JsonSchema)]
pub struct MailReadArgs {
    /// The message ID from a previous mail:check result.
    pub message_id: String,
}

pub struct MailReadTool {
    pub client: Arc<GmailClient>,
}

#[async_trait]
impl Tool for MailReadTool {
    fn name(&self) -> &'static str {
        "mail:read"
    }

    fn description(&self) -> &'static str {
        "Read full content of a Gmail message by ID (from mail:check). Returns parsed headers (From, To, Subject, Date) and body text."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MailReadArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet { max_chars: 600 }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MailReadArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let id = parsed.message_id.trim();
        if id.is_empty() {
            return Err(FcpError::SchemaViolation(
                "message_id must be non-empty".into(),
            ));
        }

        let raw = self.client.get_message(id).await?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "Gmail message response is not valid JSON");
            FcpError::ToolFault {
                tool_name: "mail:read".into(),
                reason: "unexpected Gmail API response (invalid JSON)".into(),
            }
        })?;

        let out = format_message_from_json(&v);
        if let Err(e) = self.client.mark_message_read(id, "mail:read").await {
            tracing::warn!(
                error = %e,
                message_id = %id,
                "mail:read could not mark message as read; body still returned"
            );
        } else {
            tracing::debug!(message_id = %id, "mail:read marked message as read");
        }
        Ok(out)
    }
}

fn format_message_from_json(v: &Value) -> String {
    let mut out = String::from("[mail:read]\n");

    if let Some(snippet) = v.get("snippet").and_then(|s| s.as_str()) {
        out.push_str(&format!("Snippet: {snippet}\n"));
    }

    if let Some(payload) = v.get("payload") {
        if let Some(headers) = payload.get("headers").and_then(|h| h.as_array()) {
            for key in ["From", "To", "Subject", "Date", "Cc"] {
                if let Some(val) = header_value_from_json(headers, key) {
                    out.push_str(&format!("{key}: {val}\n"));
                }
            }
        }
    }

    out.push('\n');

    let body = v
        .get("payload")
        .map(|p| extract_best_body_text(p))
        .unwrap_or_default();

    if body.is_empty() {
        out.push_str("(no text body available)");
    } else if body.len() > MAX_BODY_CHARS {
        out.push_str(&body[..MAX_BODY_CHARS]);
        out.push_str("\n\n[TRUNCATED]");
    } else {
        out.push_str(&body);
    }

    out
}

fn header_value_from_json(headers: &[Value], name: &str) -> Option<String> {
    for h in headers {
        let n = h.get("name").and_then(|x| x.as_str())?;
        if n.eq_ignore_ascii_case(name) {
            return h.get("value").and_then(|x| x.as_str()).map(String::from);
        }
    }
    None
}

/// Prefer `text/plain`, then `text/html` (converted), walking nested `parts`.
fn extract_best_body_text(part: &Value) -> String {
    if let Some(t) = find_mime_body(part, "text/plain") {
        return t;
    }
    if let Some(html) = find_mime_body(part, "text/html") {
        return html_to_plain(&html);
    }
    String::new()
}

fn find_mime_body(part: &Value, mime: &str) -> Option<String> {
    let m = part.get("mimeType").and_then(|x| x.as_str())?;
    if m == mime {
        return decode_part_body_to_text(part);
    }
    if let Some(parts) = part.get("parts").and_then(|p| p.as_array()) {
        for child in parts {
            if let Some(t) = find_mime_body(child, mime) {
                return Some(t);
            }
        }
    }
    None
}

fn decode_part_body_to_text(part: &Value) -> Option<String> {
    let data = part.get("body")?.get("data")?.as_str()?;
    let bytes = decode_gmail_base64(data)?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// Gmail uses base64url for `body.data`; generated types used standard base64 and failed on `-`.
fn decode_gmail_base64(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let try_decode = |buf: &str| {
        URL_SAFE_NO_PAD
            .decode(buf.as_bytes())
            .or_else(|_| URL_SAFE.decode(buf.as_bytes()))
            .or_else(|_| STANDARD_NO_PAD.decode(buf.as_bytes()))
            .or_else(|_| STANDARD.decode(buf.as_bytes()))
    };
    if let Ok(b) = try_decode(&s) {
        return Some(b);
    }
    let pad = (4 - s.len() % 4) % 4;
    let padded = format!("{}{}", s, "=".repeat(pad));
    try_decode(&padded).ok()
}

fn html_to_plain(html: &str) -> String {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "noscript"])
        .build();
    converter.convert(html).unwrap_or_else(|_| html.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mail_read_args_schema_is_valid() {
        let schema = schemars::schema_for!(MailReadArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }

    #[test]
    fn header_value_from_json_case_insensitive() {
        let headers = vec![
            json!({"name": "From", "value": "a@b.com"}),
            json!({"name": "Subject", "value": "Hi"}),
        ];
        assert_eq!(
            header_value_from_json(&headers, "from").as_deref(),
            Some("a@b.com")
        );
        assert_eq!(
            header_value_from_json(&headers, "subject").as_deref(),
            Some("Hi")
        );
        assert!(header_value_from_json(&headers, "Bcc").is_none());
    }

    #[test]
    fn decode_gmail_base64_url_safe() {
        let raw = "SGVsbG8gd29ybGQ";
        let bytes = decode_gmail_base64(raw).expect("decode");
        assert_eq!(bytes, b"Hello world");
    }

    #[test]
    fn decode_gmail_base64_with_dash_url_chars() {
        let s = "eyJ0ZXN0IjogdHJ1ZX0";
        let bytes = decode_gmail_base64(s).expect("decode url-safe");
        assert_eq!(String::from_utf8_lossy(&bytes), "{\"test\": true}");
    }

    #[test]
    fn format_message_from_json_minimal() {
        let v = json!({
            "snippet": "Quick preview",
            "payload": {
                "mimeType": "text/plain",
                "headers": [
                    {"name": "From", "value": "x@y.com"},
                    {"name": "Subject", "value": "Subj"}
                ],
                "body": {
                    "data": "SGVsbG8="
                }
            }
        });
        let out = format_message_from_json(&v);
        assert!(out.contains("[mail:read]"));
        assert!(out.contains("From: x@y.com"));
        assert!(out.contains("Subject: Subj"));
        assert!(out.contains("Hello"));
    }

    #[test]
    fn extract_best_prefers_plain_over_html() {
        let part = json!({
            "mimeType": "multipart/alternative",
            "parts": [
                {
                    "mimeType": "text/html",
                    "body": { "data": "PGI+aGk8L2I+" }
                },
                {
                    "mimeType": "text/plain",
                    "body": { "data": "aGVsbG8=" }
                }
            ]
        });
        let t = extract_best_body_text(&part);
        assert_eq!(t, "hello");
    }
}
