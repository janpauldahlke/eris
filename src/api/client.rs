use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::api::template::apply_template;
use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};

const TRUNCATION_SUFFIX: &str = "\n\n[SYSTEM: RESPONSE TRUNCATED DUE TO LENGTH LIMITS]";

/// Internal HTTP client for [`ApiProfile`] entries in [`AppConfig::apis`]. Not an LLM tool.
pub struct ApiHttpClient {
    client: reqwest::Client,
    config: Arc<AppConfig>,
}

impl ApiHttpClient {
    pub fn new(config: Arc<AppConfig>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.web_fetch_timeout_secs))
            .user_agent(concat!("eris/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| {
                tracing::error!(error = %e, "failed to build reqwest client for ApiHttpClient");
                FcpError::NetworkFault("HTTP client initialization failed".into())
            })?;
        Ok(Self { client, config })
    }

    /// GET request: templated URL, query, and headers; returns truncated UTF-8 body on HTTP 2xx only.
    pub async fn get_templated(
        &self,
        profile_id: &str,
        params: &HashMap<String, String>,
    ) -> Result<String> {
        let profile = self
            .config
            .apis
            .get(profile_id)
            .ok_or_else(|| FcpError::Config(format!("unknown API profile: {profile_id}")))?;
        if !profile.enabled {
            return Err(FcpError::Config(format!("API profile disabled: {profile_id}")));
        }

        let base_url_str = apply_template(&profile.base_url, params)?;
        let mut url = url::Url::parse(&base_url_str)
            .map_err(|e| FcpError::Config(format!("invalid base URL after template: {e}")))?;

        for (k, v) in &profile.query {
            let key_t = apply_template(k, params)?;
            let val_t = apply_template(v, params)?;
            url.query_pairs_mut().append_pair(&key_t, &val_t);
        }

        let mut headers = HeaderMap::new();
        for (k, v) in &profile.headers {
            let name_s = apply_template(k, params)?;
            let val_s = apply_template(v, params)?;
            let name = HeaderName::from_bytes(name_s.as_bytes()).map_err(|_| {
                FcpError::Config(format!("invalid HTTP header name in API profile: {name_s}"))
            })?;
            let value = HeaderValue::from_str(&val_s).map_err(|_| {
                FcpError::Config("invalid HTTP header value in API profile".into())
            })?;
            headers.insert(name, value);
        }

        let max_bytes = profile
            .max_response_bytes
            .unwrap_or(self.config.web_fetch_max_bytes);

        let host_for_log = url.host_str().unwrap_or("?");
        let path_for_log = url.path();
        tracing::debug!(
            profile_id,
            host = %host_for_log,
            path = %path_for_log,
            "api GET (query redacted)"
        );

        let response = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, profile_id, "API HTTP request failed");
                FcpError::NetworkFault("upstream service unreachable".into())
            })?;

        let status = response.status();
        if !status.is_success() {
            tracing::warn!(
                status = %status.as_u16(),
                profile_id,
                "Upstream API non-success"
            );
            return Err(FcpError::ToolFault {
                tool_name: "api_client".into(),
                reason: "upstream service returned an error".into(),
            });
        }

        let bytes = response.bytes().await.map_err(|e| {
            tracing::error!(error = %e, profile_id, "reading API response body failed");
            FcpError::NetworkFault("failed to read upstream response".into())
        })?;

        let truncated = bytes.len() > max_bytes;
        let slice = if truncated {
            tracing::warn!(
                len = bytes.len(),
                max = max_bytes,
                profile_id,
                "API response truncated"
            );
            truncate_utf8_prefix(&bytes, max_bytes)
        } else {
            bytes.as_ref()
        };

        let mut text = String::from_utf8(slice.to_vec()).map_err(|_| FcpError::ToolFault {
            tool_name: "api_client".into(),
            reason: "upstream response body is not valid UTF-8".into(),
        })?;
        if truncated {
            text.push_str(TRUNCATION_SUFFIX);
        }
        Ok(text)
    }
}

/// Truncate `bytes` to at most `max` bytes without splitting a UTF-8 codepoint.
fn truncate_utf8_prefix(bytes: &[u8], max: usize) -> &[u8] {
    if bytes.len() <= max {
        return bytes;
    }
    let mut end = max;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    &bytes[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ApiProfile;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn get_templated_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("q", "hello"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&server)
            .await;

        let mut apis = HashMap::new();
        apis.insert(
            "test".into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{}/search", server.uri()),
                query: [("q".into(), "{term}".into())].into_iter().collect(),
                headers: HashMap::new(),
                max_response_bytes: None,
                stale_after_secs: None,
            },
        );

        let cfg = AppConfig {
            apis,
            ..Default::default()
        };
        let client = ApiHttpClient::new(Arc::new(cfg)).expect("client");

        let mut params = HashMap::new();
        params.insert("term".into(), "hello".into());

        let body = client
            .get_templated("test", &params)
            .await
            .expect("ok");
        assert_eq!(body, r#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn get_templated_404_is_tool_fault() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let mut apis = HashMap::new();
        apis.insert(
            "test".into(),
            ApiProfile {
                base_url: format!("{}/missing", server.uri()),
                query: HashMap::new(),
                headers: HashMap::new(),
                enabled: true,
                max_response_bytes: None,
                stale_after_secs: None,
            },
        );

        let cfg = AppConfig {
            apis,
            ..Default::default()
        };
        let client = ApiHttpClient::new(Arc::new(cfg)).expect("client");

        let r = client.get_templated("test", &HashMap::new()).await;
        assert!(matches!(
            r,
            Err(FcpError::ToolFault {
                tool_name,
                reason,
            }) if tool_name == "api_client" && reason == "upstream service returned an error"
        ));
    }

    #[tokio::test]
    async fn get_templated_missing_param_is_config_error() {
        let server = MockServer::start().await;
        let mut apis = HashMap::new();
        apis.insert(
            "test".into(),
            ApiProfile {
                base_url: format!("{}/x", server.uri()),
                query: [("q".into(), "{term}".into())].into_iter().collect(),
                headers: HashMap::new(),
                enabled: true,
                max_response_bytes: None,
                stale_after_secs: None,
            },
        );
        let cfg = AppConfig {
            apis,
            ..Default::default()
        };
        let client = ApiHttpClient::new(Arc::new(cfg)).expect("client");

        let r = client.get_templated("test", &HashMap::new()).await;
        assert!(matches!(r, Err(FcpError::Config(_))));
    }

    #[tokio::test]
    async fn get_templated_truncates_body() {
        let server = MockServer::start().await;
        let body = "a".repeat(100);
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.clone()))
            .mount(&server)
            .await;

        let mut apis = HashMap::new();
        apis.insert(
            "test".into(),
            ApiProfile {
                base_url: format!("{}/big", server.uri()),
                query: HashMap::new(),
                headers: HashMap::new(),
                enabled: true,
                max_response_bytes: Some(20),
                stale_after_secs: None,
            },
        );
        let cfg = AppConfig {
            apis,
            ..Default::default()
        };
        let client = ApiHttpClient::new(Arc::new(cfg)).expect("client");

        let out = client
            .get_templated("test", &HashMap::new())
            .await
            .expect("ok");
        assert!(out.starts_with("aaaaaaaaaaaaaaaaaaaa"));
        assert!(out.contains(TRUNCATION_SUFFIX));
    }
}
