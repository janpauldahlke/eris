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
pub struct MailWriteArgs {
    /// Recipient email address.
    pub to: String,
    /// Email subject line.
    pub subject: String,
    /// Plain text email body.
    pub body: String,
    /// Optional CC recipient.
    #[serde(default)]
    pub cc: Option<String>,
    /// Optional BCC recipient.
    #[serde(default)]
    pub bcc: Option<String>,
}

pub struct MailWriteTool {
    pub client: Arc<GmailClient>,
}

#[async_trait]
impl Tool for MailWriteTool {
    fn name(&self) -> &'static str {
        "mail:write"
    }

    fn description(&self) -> &'static str {
        "Send an email via Gmail. Requires to, subject, and body. Optionally cc and bcc."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(MailWriteArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::MarkerOnly
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: MailWriteArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let to = parsed.to.trim();
        if to.is_empty() || !is_plausible_email(to) {
            return Err(FcpError::SchemaViolation(
                "to must be a valid email address".into(),
            ));
        }
        if parsed.subject.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "subject must be non-empty".into(),
            ));
        }
        if parsed.body.trim().is_empty() {
            return Err(FcpError::SchemaViolation("body must be non-empty".into()));
        }
        if let Some(cc) = &parsed.cc {
            if !cc.trim().is_empty() && !is_plausible_email(cc.trim()) {
                return Err(FcpError::SchemaViolation(
                    "cc must be a valid email address".into(),
                ));
            }
        }
        if let Some(bcc) = &parsed.bcc {
            if !bcc.trim().is_empty() && !is_plausible_email(bcc.trim()) {
                return Err(FcpError::SchemaViolation(
                    "bcc must be a valid email address".into(),
                ));
            }
        }

        let raw = self
            .client
            .send_message(
                to,
                parsed.subject.trim(),
                parsed.body.trim(),
                parsed.cc.as_deref().filter(|s| !s.trim().is_empty()),
                parsed.bcc.as_deref().filter(|s| !s.trim().is_empty()),
            )
            .await?;

        let msg_id = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(String::from))
            .unwrap_or_else(|| "unknown".into());

        Ok(format!(
            "[mail:write] Message sent successfully (id={msg_id}) to {to}."
        ))
    }
}

fn is_plausible_email(s: &str) -> bool {
    let parts: Vec<&str> = s.splitn(2, '@').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && parts[1].contains('.')
        && !parts[1].starts_with('.')
        && !parts[1].ends_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_write_args_schema_is_valid() {
        let schema = schemars::schema_for!(MailWriteArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }

    #[test]
    fn is_plausible_email_accepts_valid() {
        assert!(is_plausible_email("user@example.com"));
        assert!(is_plausible_email("a@b.co"));
        assert!(is_plausible_email("test+tag@sub.domain.org"));
    }

    #[test]
    fn is_plausible_email_rejects_invalid() {
        assert!(!is_plausible_email(""));
        assert!(!is_plausible_email("noatsign"));
        assert!(!is_plausible_email("@example.com"));
        assert!(!is_plausible_email("user@"));
        assert!(!is_plausible_email("user@.com"));
        assert!(!is_plausible_email("user@com."));
        assert!(!is_plausible_email("user@nodot"));
    }
}
