use std::time::Duration;

use base64::Engine;

use crate::config::GoogleConfig;
use crate::executive::error::{FcpError, Result};
use super::auth::GoogleAuth;

const GMAIL_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
/// Must be a subset of scopes allowed for this service account in Admin → Domain-wide delegation.
/// We use full mail scope only; do not add scopes here that are not listed there or token exchange returns 401.
const GMAIL_SCOPES: &[&str] = &["https://mail.google.com/"];

pub struct GmailClient {
    auth: GoogleAuth,
    http: reqwest::Client,
}

impl GmailClient {
    pub async fn new(config: &GoogleConfig) -> Result<Option<Self>> {
        if !config.enabled {
            tracing::info!("Google integration disabled in config");
            return Ok(None);
        }
        let key_path = config.service_account_key.as_ref().ok_or_else(|| {
            FcpError::Config("google.enabled=true but service_account_key is missing".into())
        })?;
        let user = config.impersonate_user.as_deref().ok_or_else(|| {
            FcpError::Config("google.enabled=true but impersonate_user is missing".into())
        })?;
        let auth =
            GoogleAuth::from_service_account_key(key_path, user, GMAIL_SCOPES).await?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("gmail http client: {e}")))?;

        tracing::info!(
            impersonate = %user,
            "GmailClient initialized"
        );
        Ok(Some(Self { auth, http }))
    }

    pub async fn list_messages(
        &self,
        query: Option<&str>,
        max_results: u32,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let mut url = format!("{GMAIL_BASE}/messages?maxResults={max_results}");
        if let Some(q) = query {
            url.push_str(&format!(
                "&q={}",
                percent_encoding::utf8_percent_encode(q, percent_encoding::NON_ALPHANUMERIC)
            ));
        }
        self.gmail_get(&url, &token, "mail:check").await
    }

    pub async fn get_message(&self, message_id: &str) -> Result<String> {
        let token = self.auth.access_token().await?;
        let id = encode_path_segment(message_id);
        let url = format!("{GMAIL_BASE}/messages/{id}?format=full");
        self.gmail_get(&url, &token, "mail:read").await
    }

    /// Lightweight message row for inbox lists: headers + snippet, no body (`format=metadata`).
    pub async fn get_message_metadata(&self, message_id: &str) -> Result<String> {
        let token = self.auth.access_token().await?;
        let id = encode_path_segment(message_id);
        let url = format!(
            "{GMAIL_BASE}/messages/{id}?format=metadata&metadataHeaders=Subject&metadataHeaders=From&metadataHeaders=Date"
        );
        self.gmail_get(&url, &token, "mail:check").await
    }

    pub async fn send_message(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        cc: Option<&str>,
        bcc: Option<&str>,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let rfc2822 = build_rfc2822(to, subject, body, cc, bcc);
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rfc2822.as_bytes());

        let url = format!("{GMAIL_BASE}/messages/send");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "raw": raw }))
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Gmail send request failed");
                FcpError::ToolFault {
                    tool_name: "mail:write".into(),
                    reason: "Gmail API unreachable".into(),
                }
            })?;

        handle_gmail_response(resp, "mail:write").await
    }

    async fn gmail_get(&self, url: &str, token: &str, tool_name: &str) -> Result<String> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Gmail GET request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Gmail API unreachable".into(),
                }
            })?;
        handle_gmail_response(resp, tool_name).await
    }
}

fn encode_path_segment(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

async fn handle_gmail_response(resp: reqwest::Response, tool_name: &str) -> Result<String> {
    let status = resp.status();
    let body = resp.text().await.map_err(|e| {
        tracing::error!(error = %e, tool = tool_name, "reading Gmail response body");
        FcpError::ToolFault {
            tool_name: tool_name.into(),
            reason: "failed to read Gmail response".into(),
        }
    })?;

    if status.is_success() {
        return Ok(body);
    }

    tracing::warn!(
        status = %status.as_u16(),
        tool = tool_name,
        body_preview = %body.chars().take(200).collect::<String>(),
        "Gmail API error"
    );

    let reason = match status.as_u16() {
        401 => "Gmail authentication failed — check service account credentials and domain-wide delegation",
        403 => "Gmail authorization denied — service account may lack required scopes or delegation",
        404 => "Gmail resource not found — message ID may be invalid",
        429 => "Gmail rate limit exceeded — please wait before retrying",
        _ => "Gmail API returned an error",
    };
    Err(FcpError::ToolFault {
        tool_name: tool_name.into(),
        reason: reason.into(),
    })
}

fn build_rfc2822(
    to: &str,
    subject: &str,
    body: &str,
    cc: Option<&str>,
    bcc: Option<&str>,
) -> String {
    let mut msg = format!("To: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n");
    if let Some(cc_val) = cc {
        msg.push_str(&format!("Cc: {cc_val}\r\n"));
    }
    if let Some(bcc_val) = bcc {
        msg.push_str(&format!("Bcc: {bcc_val}\r\n"));
    }
    msg.push_str(&format!("\r\n{body}"));
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_rfc2822_basic() {
        let msg = build_rfc2822("a@b.com", "Hello", "World", None, None);
        assert!(msg.contains("To: a@b.com"));
        assert!(msg.contains("Subject: Hello"));
        assert!(msg.contains("Content-Type: text/plain"));
        assert!(msg.ends_with("\r\nWorld"));
        assert!(!msg.contains("Cc:"));
        assert!(!msg.contains("Bcc:"));
    }

    #[test]
    fn build_rfc2822_with_cc_bcc() {
        let msg = build_rfc2822("a@b.com", "Hi", "Body", Some("c@d.com"), Some("e@f.com"));
        assert!(msg.contains("Cc: c@d.com"));
        assert!(msg.contains("Bcc: e@f.com"));
    }

    #[test]
    fn rfc2822_base64url_roundtrip() {
        let msg = build_rfc2822("test@example.com", "Test", "Body text", None, None);
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(msg.as_bytes());
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&encoded)
            .expect("decode");
        assert_eq!(String::from_utf8(decoded).expect("utf8"), msg);
    }

    #[tokio::test]
    async fn gmail_client_none_when_disabled() {
        let config = GoogleConfig {
            enabled: false,
            ..Default::default()
        };
        let client = GmailClient::new(&config).await.expect("no error");
        assert!(client.is_none());
    }

    #[tokio::test]
    async fn gmail_client_errors_on_missing_key() {
        let config = GoogleConfig {
            enabled: true,
            service_account_key: None,
            impersonate_user: Some("user@example.com".into()),
        };
        let result = GmailClient::new(&config).await;
        assert!(matches!(result, Err(FcpError::Config(_))));
    }

    #[tokio::test]
    async fn gmail_client_errors_on_missing_user() {
        let config = GoogleConfig {
            enabled: true,
            service_account_key: Some("/tmp/fake.json".into()),
            impersonate_user: None,
        };
        let result = GmailClient::new(&config).await;
        assert!(matches!(result, Err(FcpError::Config(_))));
    }
}
