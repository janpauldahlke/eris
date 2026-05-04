use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;

use crate::config::MoltbookConfig;
use crate::executive::error::{FcpError, Result};

pub const MOLTBOOK_PROD_BASE_URL: &str = "https://www.moltbook.com/api/v1";
const TOOL_NAME: &str = "moltbook";
const TRUNCATED_SUFFIX: &str = "\n\n[SYSTEM: MOLTBOOK RESPONSE TRUNCATED]";

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MoltbookRateLimit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MoltbookResponse {
    pub body: Value,
    pub rate_limit: MoltbookRateLimit,
}

#[derive(Clone, Debug)]
pub struct MoltbookClient {
    client: reqwest::Client,
    base_url: Url,
    api_key: Option<String>,
    max_response_bytes: usize,
    #[cfg(test)]
    allow_non_prod_auth: bool,
}

impl MoltbookClient {
    pub fn unauthenticated(config: &MoltbookConfig, timeout_secs: u64, max_response_bytes: usize) -> Result<Self> {
        let base_url = parse_and_validate_prod_base_url(&config.base_url)?;
        let client = build_http_client(timeout_secs)?;
        Ok(Self {
            client,
            base_url,
            api_key: None,
            max_response_bytes,
            #[cfg(test)]
            allow_non_prod_auth: false,
        })
    }

    pub async fn authenticated(
        config: &MoltbookConfig,
        timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let base_url = parse_and_validate_prod_base_url(&config.base_url)?;
        let api_key = resolve_api_key(config).await?;
        let client = build_http_client(timeout_secs)?;
        Ok(Self {
            client,
            base_url,
            api_key: Some(api_key),
            max_response_bytes,
            #[cfg(test)]
            allow_non_prod_auth: false,
        })
    }

    #[cfg(test)]
    pub fn for_test(base_url: String, api_key: Option<String>) -> Result<Self> {
        let base_url = parse_base_url(&base_url)?;
        let client = build_http_client(5)?;
        Ok(Self {
            client,
            base_url,
            api_key,
            max_response_bytes: 65_536,
            allow_non_prod_auth: true,
        })
    }

    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, Option<String>)],
        auth: AuthMode,
    ) -> Result<MoltbookResponse> {
        self.request(Method::GET, path, query, None, auth).await
    }

    pub async fn post(
        &self,
        path: &str,
        body: Option<Value>,
        auth: AuthMode,
    ) -> Result<MoltbookResponse> {
        self.request(Method::POST, path, &[], body, auth).await
    }

    pub async fn delete(
        &self,
        path: &str,
        body: Option<Value>,
        auth: AuthMode,
    ) -> Result<MoltbookResponse> {
        self.request(Method::DELETE, path, &[], body, auth).await
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, Option<String>)],
        body: Option<Value>,
        auth: AuthMode,
    ) -> Result<MoltbookResponse> {
        let url = self.url(path, query)?;
        let mut headers = HeaderMap::new();
        if let AuthMode::Bearer = auth {
            #[cfg(test)]
            if !self.allow_non_prod_auth {
                ensure_prod_api_url(&url)?;
            }
            #[cfg(not(test))]
            ensure_prod_api_url(&url)?;
            let key = self.api_key.as_deref().ok_or_else(|| {
                FcpError::Config(
                    "Moltbook API key missing; set MOLTBOOK_API_KEY or moltbook.api_key_file"
                        .into(),
                )
            })?;
            let value = format!("Bearer {key}");
            let header = value.parse().map_err(|_| {
                FcpError::Config("Moltbook API key contains invalid header characters".into())
            })?;
            headers.insert(AUTHORIZATION, header);
        }
        if body.is_some() {
            headers.insert(CONTENT_TYPE, "application/json".parse().map_err(|_| {
                FcpError::Config("failed to build Moltbook content-type header".into())
            })?);
        }

        tracing::debug!(
            method = %method,
            host = %url.host_str().unwrap_or("?"),
            path = %url.path(),
            "moltbook request"
        );

        let mut req = self.client.request(method, url).headers(headers);
        if let Some(json_body) = body {
            req = req.json(&json_body);
        }
        let response = req.send().await.map_err(|e| {
            tracing::warn!(
                error = %e,
                is_timeout = e.is_timeout(),
                is_connect = e.is_connect(),
                "moltbook request failed"
            );
            FcpError::NetworkFault("Moltbook request failed".into())
        })?;

        let status = response.status();
        let rate_limit = rate_limit_from_headers(response.headers());
        let bytes = response.bytes().await.map_err(|e| {
            tracing::warn!(error = %e, "failed reading Moltbook response body");
            FcpError::NetworkFault("failed to read Moltbook response".into())
        })?;
        let text = truncate_utf8_lossy(&bytes, self.max_response_bytes);
        if !status.is_success() {
            return Err(map_http_error(status, &text, &rate_limit));
        }
        let body = if text.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_str(&text).map_err(FcpError::ParseFault)?
        };
        Ok(MoltbookResponse { body, rate_limit })
    }

    fn url(&self, path: &str, query: &[(&str, Option<String>)]) -> Result<Url> {
        let path = path.trim_start_matches('/');
        if path.contains("..") {
            return Err(FcpError::SchemaViolation(
                "Moltbook path cannot contain traversal segments".into(),
            ));
        }
        let mut url = self.base_url.clone();
        let base_path = url.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}/{path}"));
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                if let Some(value) = value {
                    pairs.append_pair(key, value);
                }
            }
        }
        Ok(url)
    }
}

#[derive(Clone, Copy)]
pub enum AuthMode {
    None,
    Bearer,
}

pub fn clean_path_segment(label: &str, raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('?') || trimmed.contains('#') {
        return Err(FcpError::SchemaViolation(format!(
            "{label} must be a non-empty path segment"
        )));
    }
    Ok(trimmed.to_string())
}

pub fn tool_result(tool: &str, response: MoltbookResponse, next_step_hint: impl Into<String>) -> Result<String> {
    let mut out = serde_json::Map::new();
    out.insert("tool".into(), Value::String(tool.to_string()));
    out.insert("data".into(), response.body);
    out.insert(
        "rate_limit".into(),
        serde_json::to_value(response.rate_limit).map_err(FcpError::ParseFault)?,
    );
    out.insert("next_step_hint".into(), Value::String(next_step_hint.into()));
    serde_json::to_string(&Value::Object(out)).map_err(FcpError::ParseFault)
}

pub fn validate_content_len(label: &str, value: &str, min: usize, max: usize) -> Result<String> {
    let trimmed = value.trim();
    let len = trimmed.chars().count();
    if len < min || len > max {
        return Err(FcpError::SchemaViolation(format!(
            "{label} must be between {min} and {max} characters"
        )));
    }
    Ok(trimmed.to_string())
}

fn build_http_client(timeout_secs: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .user_agent(concat!("eris/", env!("CARGO_PKG_VERSION"), " moltbook-tool"))
        .build()
        .map_err(|e| {
            tracing::error!(error = %e, "failed building Moltbook HTTP client");
            FcpError::NetworkFault("Moltbook HTTP client initialization failed".into())
        })
}

fn parse_base_url(raw: &str) -> Result<Url> {
    let trimmed = raw.trim().trim_end_matches('/');
    Url::parse(trimmed).map_err(|e| FcpError::Config(format!("invalid Moltbook base URL: {e}")))
}

fn parse_and_validate_prod_base_url(raw: &str) -> Result<Url> {
    let url = parse_base_url(raw)?;
    ensure_prod_api_url(&url)?;
    Ok(url)
}

fn ensure_prod_api_url(url: &Url) -> Result<()> {
    if url.scheme() == "https"
        && url.host_str() == Some("www.moltbook.com")
        && url.path().starts_with("/api/v1")
    {
        return Ok(());
    }
    Err(FcpError::Config(
        "Moltbook authenticated requests are pinned to https://www.moltbook.com/api/v1"
            .into(),
    ))
}

async fn resolve_api_key(config: &MoltbookConfig) -> Result<String> {
    let env_name = config.api_key_env.trim();
    if !env_name.is_empty() {
        if let Ok(value) = std::env::var(env_name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    if let Some(path) = &config.api_key_file {
        let resolved = expand_home(path);
        let bytes = tokio::fs::read(&resolved).await.map_err(|e| {
            FcpError::Config(format!(
                "failed reading Moltbook credentials file {}: {e}",
                resolved.display()
            ))
        })?;
        let creds: MoltbookCredentials = serde_json::from_slice(&bytes).map_err(FcpError::ParseFault)?;
        let trimmed = creds.api_key.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err(FcpError::Config(
        "Moltbook API key missing; set MOLTBOOK_API_KEY or moltbook.api_key_file".into(),
    ))
}

fn expand_home(path: &Path) -> PathBuf {
    let as_str = path.to_string_lossy();
    if let Some(rest) = as_str.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

#[derive(Deserialize)]
struct MoltbookCredentials {
    api_key: String,
    #[allow(dead_code)]
    agent_name: Option<String>,
}

fn rate_limit_from_headers(headers: &HeaderMap) -> MoltbookRateLimit {
    MoltbookRateLimit {
        limit: header_to_string(headers, "x-ratelimit-limit"),
        remaining: header_to_string(headers, "x-ratelimit-remaining"),
        reset: header_to_string(headers, "x-ratelimit-reset"),
        retry_after: header_to_string(headers, "retry-after"),
    }
}

fn header_to_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string)
}

fn map_http_error(status: StatusCode, body: &str, rate_limit: &MoltbookRateLimit) -> FcpError {
    let reason = summarize_error_body(body);
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            FcpError::Config(format!("Moltbook authentication failed: {reason}"))
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let retry = rate_limit
                .retry_after
                .as_deref()
                .map(|v| format!(" retry_after={v}s"))
                .unwrap_or_default();
            FcpError::ToolFault {
                tool_name: TOOL_NAME.into(),
                reason: format!("Moltbook rate limit or cooldown hit.{retry} {reason}"),
            }
        }
        _ => FcpError::ToolFault {
            tool_name: TOOL_NAME.into(),
            reason: format!("Moltbook API returned HTTP {}: {reason}", status.as_u16()),
        },
    }
}

fn summarize_error_body(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        let mut parts = BTreeMap::new();
        for key in ["error", "message", "hint", "retry_after_seconds", "retry_after_minutes"] {
            if let Some(v) = value.get(key) {
                parts.insert(key, v.to_string());
            }
        }
        if !parts.is_empty() {
            return parts
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
        }
    }
    body.chars().take(280).collect()
}

fn truncate_utf8_lossy(bytes: &[u8], max: usize) -> String {
    let mut text = if bytes.len() <= max {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        let mut end = max;
        while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
            end -= 1;
        }
        let mut text = String::from_utf8_lossy(&bytes[..end]).to_string();
        text.push_str(TRUNCATED_SUFFIX);
        text
    };
    if text.len() > max + TRUNCATED_SUFFIX.len() {
        text.truncate(max + TRUNCATED_SUFFIX.len());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_www_prod_base_url() {
        let cfg = MoltbookConfig {
            enabled: true,
            base_url: "https://moltbook.com/api/v1".into(),
            ..MoltbookConfig::default()
        };
        let err = MoltbookClient::unauthenticated(&cfg, 5, 1024).expect_err("must reject");
        assert!(matches!(err, FcpError::Config(_)));
    }

    #[test]
    fn clean_path_segment_rejects_slashes() {
        let err = clean_path_segment("post_id", "a/b").expect_err("slash should fail");
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }
}
