use std::sync::Arc;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;

use super::auth::GoogleAuth;
use crate::executive::error::{FcpError, Result};

const CALENDAR_BASE: &str = "https://www.googleapis.com/calendar/v3";

pub struct CalendarClient {
    auth: Arc<GoogleAuth>,
    http: reqwest::Client,
}

impl CalendarClient {
    pub fn from_auth(auth: Arc<GoogleAuth>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| FcpError::NetworkFault(format!("calendar http client: {e}")))?;
        tracing::info!("CalendarClient initialized");
        Ok(Self { auth, http })
    }

    /// `events.list` — optional RFC3339 `time_min` / `time_max`; uses `singleEvents=true` and `orderBy=startTime`.
    pub async fn list_events(
        &self,
        calendar_id: &str,
        time_min: Option<&str>,
        time_max: Option<&str>,
        max_results: u32,
        tool_name: &str,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let cal = encode_path_segment(calendar_id);
        let mut url = format!(
            "{CALENDAR_BASE}/calendars/{cal}/events?singleEvents=true&orderBy=startTime&maxResults={max_results}"
        );
        if let Some(t) = time_min {
            url.push_str(&format!(
                "&timeMin={}",
                percent_encoding::utf8_percent_encode(t, percent_encoding::NON_ALPHANUMERIC)
            ));
        }
        if let Some(t) = time_max {
            url.push_str(&format!(
                "&timeMax={}",
                percent_encoding::utf8_percent_encode(t, percent_encoding::NON_ALPHANUMERIC)
            ));
        }
        self.calendar_get(&url, &token, tool_name).await
    }

    pub async fn get_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        tool_name: &str,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let cal = encode_path_segment(calendar_id);
        let ev = encode_path_segment(event_id);
        let url = format!("{CALENDAR_BASE}/calendars/{cal}/events/{ev}");
        self.calendar_get(&url, &token, tool_name).await
    }

    pub async fn insert_event(
        &self,
        calendar_id: &str,
        body: &serde_json::Value,
        tool_name: &str,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let cal = encode_path_segment(calendar_id);
        let url = format!("{CALENDAR_BASE}/calendars/{cal}/events");
        self.calendar_post_json(&url, &token, body, tool_name).await
    }

    pub async fn patch_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        body: &serde_json::Value,
        tool_name: &str,
    ) -> Result<String> {
        let token = self.auth.access_token().await?;
        let cal = encode_path_segment(calendar_id);
        let ev = encode_path_segment(event_id);
        let url = format!("{CALENDAR_BASE}/calendars/{cal}/events/{ev}");
        self.calendar_patch_json(&url, &token, body, tool_name)
            .await
    }

    pub async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        tool_name: &str,
    ) -> Result<()> {
        let token = self.auth.access_token().await?;
        let cal = encode_path_segment(calendar_id);
        let ev = encode_path_segment(event_id);
        let url = format!("{CALENDAR_BASE}/calendars/{cal}/events/{ev}");
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Calendar DELETE request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Google Calendar API unreachable".into(),
                }
            })?;
        let status = resp.status();
        if status.as_u16() == 204 || status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.map_err(|e| {
            tracing::error!(error = %e, tool = tool_name, "reading Calendar DELETE response body");
            FcpError::ToolFault {
                tool_name: tool_name.into(),
                reason: "failed to read Calendar response".into(),
            }
        })?;
        tracing::warn!(
            status = %status.as_u16(),
            tool = tool_name,
            body_preview = %body.chars().take(200).collect::<String>(),
            "Calendar API error"
        );
        Err(FcpError::ToolFault {
            tool_name: tool_name.into(),
            reason: calendar_error_reason(status.as_u16()).into(),
        })
    }

    async fn calendar_get(&self, url: &str, token: &str, tool_name: &str) -> Result<String> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Calendar GET request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Google Calendar API unreachable".into(),
                }
            })?;
        handle_calendar_response(resp, tool_name).await
    }

    async fn calendar_post_json(
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
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Calendar POST request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Google Calendar API unreachable".into(),
                }
            })?;
        handle_calendar_response(resp, tool_name).await
    }

    async fn calendar_patch_json(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
        tool_name: &str,
    ) -> Result<String> {
        let resp = self
            .http
            .patch(url)
            .bearer_auth(token)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, tool = tool_name, "Calendar PATCH request failed");
                FcpError::ToolFault {
                    tool_name: tool_name.into(),
                    reason: "Google Calendar API unreachable".into(),
                }
            })?;
        handle_calendar_response(resp, tool_name).await
    }
}

fn encode_path_segment(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

async fn handle_calendar_response(resp: reqwest::Response, tool_name: &str) -> Result<String> {
    let status = resp.status();
    let body = resp.text().await.map_err(|e| {
        tracing::error!(error = %e, tool = tool_name, "reading Calendar response body");
        FcpError::ToolFault {
            tool_name: tool_name.into(),
            reason: "failed to read Calendar response".into(),
        }
    })?;

    if status.is_success() {
        return Ok(body);
    }

    tracing::warn!(
        status = %status.as_u16(),
        tool = tool_name,
        body_preview = %body.chars().take(200).collect::<String>(),
        "Calendar API error"
    );

    Err(FcpError::ToolFault {
        tool_name: tool_name.into(),
        reason: calendar_error_reason(status.as_u16()).into(),
    })
}

fn calendar_error_reason(status: u16) -> &'static str {
    match status {
        401 => {
            "Calendar authentication failed — check service account credentials and domain-wide delegation (include https://www.googleapis.com/auth/calendar)"
        }
        403 => {
            "Calendar authorization denied — verify Calendar API is enabled and scopes are delegated"
        }
        404 => "Calendar or event not found — check calendar_id and event_id",
        429 => "Calendar rate limit exceeded — wait before retrying",
        _ => "Google Calendar API returned an error",
    }
}
