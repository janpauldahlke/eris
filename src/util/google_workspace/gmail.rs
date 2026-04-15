use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use reqwest::header::{HeaderValue, CONTENT_LENGTH};

use crate::config::GoogleConfig;
use crate::executive::error::{FcpError, Result};
use super::auth::GoogleAuth;

const GMAIL_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
/// Gmail system label id for unread state.
pub const GMAIL_LABEL_UNREAD: &str = "UNREAD";

pub struct GmailClient {
    auth: Arc<GoogleAuth>,
    http: reqwest::Client,
}

impl GmailClient {
    pub async fn new(config: &GoogleConfig) -> Result<Option<Self>> {
        let Some(auth) = super::workspace_auth(config).await? else {
            return Ok(None);
        };
        tracing::info!("GmailClient initialized");
        Self::from_auth(auth).map(Some)
    }

    pub fn from_auth(auth: Arc<GoogleAuth>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("gmail http client: {e}")))?;
        Ok(Self { auth, http })
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

    /// Apply label changes to a message (`users.messages.modify`).
    pub async fn modify_message(
        &self,
        message_id: &str,
        add_label_ids: &[String],
        remove_label_ids: &[String],
        tool_name: &str,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let id = encode_path_segment(message_id);
        let url = format!("{GMAIL_BASE}/messages/{id}/modify");
        let body = serde_json::json!({
            "addLabelIds": add_label_ids,
            "removeLabelIds": remove_label_ids,
        });
        self.gmail_post_json(&url, &token, &body, tool_name).await
    }

    /// Remove the `UNREAD` label (mark as read). Used after a successful full read.
    pub async fn mark_message_read(&self, message_id: &str, tool_name: &str) -> Result<String> {
        self.modify_message(
            message_id,
            &[],
            &[GMAIL_LABEL_UNREAD.to_string()],
            tool_name,
        )
        .await
    }

    /// Move message to Trash (`users.messages.trash`).
    pub async fn trash_message(&self, message_id: &str, tool_name: &str) -> Result<String> {
        let token = self.auth.access_token().await?;
        let id = encode_path_segment(message_id);
        let url = format!("{GMAIL_BASE}/messages/{id}/trash");
        self.gmail_post_empty(&url, &token, tool_name).await
    }

    /// Permanently delete a message (`users.messages.delete`).
    pub async fn delete_message_permanent(&self, message_id: &str, tool_name: &str) -> Result<()> {
        let token = self.auth.access_token().await?;
        let id = encode_path_segment(message_id);
        let url = format!("{GMAIL_BASE}/messages/{id}");
        self.gmail_delete(&url, &token, tool_name).await
    }

    /// List all labels (`users.labels.list`).
    pub async fn list_labels(&self, tool_name: &str) -> Result<String> {
        let token = self.auth.access_token().await?;
        let url = format!("{GMAIL_BASE}/labels");
        self.gmail_get(&url, &token, tool_name).await
    }

    /// Create a user label (`users.labels.create`).
    pub async fn create_label(&self, name: &str, tool_name: &str) -> Result<String> {
        let token = self.auth.access_token().await?;
        let url = format!("{GMAIL_BASE}/labels");
        let body = serde_json::json!({
            "name": name,
            "labelListVisibility": "labelShow",
            "messageListVisibility": "show",
        });
        self.gmail_post_json(&url, &token, &body, tool_name).await
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

    async fn gmail_post_json(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
        tool_name: &str,
    ) -> Result<String> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Gmail POST request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Gmail API unreachable".into(),
                }
            })?;
        handle_gmail_response(resp, tool_name).await
    }

    /// POST with an empty body. Google's HTTPS front ends return **411** if `Content-Length` is
    /// missing on POST; reqwest may omit it for "no body" unless we set it explicitly (see
    /// [reqwest#2240](https://github.com/seanmonstar/reqwest/issues/2240)).
    async fn gmail_post_empty(
        &self,
        url: &str,
        token: &str,
        tool_name: &str,
    ) -> Result<String> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .header(CONTENT_LENGTH, HeaderValue::from_static("0"))
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Gmail POST request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Gmail API unreachable".into(),
                }
            })?;
        handle_gmail_response(resp, tool_name).await
    }

    async fn gmail_delete(&self, url: &str, token: &str, tool_name: &str) -> Result<()> {
        let resp = self
            .http
            .delete(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Gmail DELETE request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Gmail API unreachable".into(),
                }
            })?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.map_err(|e| {
            tracing::error!(error = %e, tool = tool_name, "reading Gmail DELETE response body");
            FcpError::ToolFault {
                tool_name: tool_name.into(),
                reason: "failed to read Gmail response".into(),
            }
        })?;
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
        411 => "Gmail returned 411 Length Required — POST body/Content-Length mismatch (unexpected after client fix)",
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
