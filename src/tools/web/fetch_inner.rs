//! Shared HTTP → markdown → ephemeral artifact pipeline for [`super::fetch::WebFetchTool`] and
//! [`crate::tools::news::NewsTodayTool`].

use crate::executive::error::{FcpError, Result};
use crate::ingest::bound_chunks_and_preview;
use crate::memory::ephemeral::EphemeralMemory;
use crate::memory::semantic::SemanticBrain;
use crate::tools::web::artifact::{WebArtifact, WebOutboundLink};
use crate::tools::web::link_extract::extract_ranked_page_links;
use crate::tools::web::markdown_focus::focus_article_text;
use htmd::HtmlToMarkdown;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, REFERER, USER_AGENT,
};
use reqwest::Client;
use serde::Serialize;
use std::time::Duration;
use std::sync::Arc;

/// Default Chrome-style UA; used if config / `HeaderValue::from_str` rejects the operator string.
pub(crate) const FALLBACK_WEB_FETCH_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub(crate) fn build_web_fetch_client(timeout_secs: u64, user_agent: &str) -> Client {
    let ua = HeaderValue::from_str(user_agent.trim()).unwrap_or_else(|_| {
        HeaderValue::from_static(FALLBACK_WEB_FETCH_UA)
    });

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, ua);
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-dest"),
        HeaderValue::from_static("document"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-mode"),
        HeaderValue::from_static("navigate"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("none"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-user"),
        HeaderValue::from_static("?1"),
    );
    headers.insert(
        HeaderName::from_static("upgrade-insecure-requests"),
        HeaderValue::from_static("1"),
    );

    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .default_headers(headers)
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub(crate) fn sanitize_markdown_noise(markdown: &str) -> String {
    let mut out = Vec::new();
    for line in markdown.lines() {
        let l = line.trim();
        if l.is_empty() {
            out.push(String::new());
            continue;
        }
        let ll = l.to_lowercase();
        if ll.contains("cookie settings")
            || ll.contains("accept all cookies")
            || ll.contains("consent preferences")
            || ll.contains("subscribe")
            || ll.contains("newsletter")
            || ll.contains("advertisement")
            || ll.contains("sponsored")
            || ll.contains("privacy policy")
            || ll.contains("terms of service")
        {
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

#[derive(Clone)]
pub(crate) struct WebFetchRuntime {
    pub client: Client,
    pub max_bytes: usize,
    pub chunk_chars: usize,
    pub preview_chars: usize,
    pub artifact_ttl_secs: u64,
    pub default_referer: Option<String>,
    pub ephemeral: Arc<EphemeralMemory>,
    pub semantic: Option<Arc<SemanticBrain>>,
}

#[derive(Serialize)]
pub(crate) struct WebFetchReceiptJson {
    pub artifact_id: String,
    pub url: String,
    pub chunk_count: usize,
    pub preview_head: String,
    pub outbound_links: Vec<WebOutboundLink>,
    pub next_step_hint: String,
}

/// Successful artifact storage; callers build receipts or aggregate (e.g. `news:today`).
pub(crate) struct WebFetchStored {
    pub artifact_id: String,
    pub url: String,
    pub chunk_count: usize,
    pub preview_head: String,
    pub outbound_links: Vec<WebOutboundLink>,
}

pub(crate) enum WebFetchRunOutcome {
    /// Stored artifact; embed indexing best-effort inside `run_web_fetch`.
    Stored(WebFetchStored),
    /// Same as today: network / HTTP / empty body — plain message for the model.
    Plain(String),
}

/// Fetch `url`, convert to chunks, persist ephemeral artifact, optional semantic upsert.
pub(crate) async fn run_web_fetch(
    rt: &WebFetchRuntime,
    url: String,
    referer_arg: Option<String>,
) -> Result<WebFetchRunOutcome> {
    let referer_raw = referer_arg
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| rt.default_referer.clone());

    let mut req = rt.client.get(&url);
    if let Some(raw) = referer_raw.as_ref() {
        if raw.starts_with("http://") || raw.starts_with("https://") {
            match HeaderValue::from_str(raw) {
                Ok(v) => {
                    req = req.header(REFERER, v);
                }
                Err(_) => {
                    tracing::warn!("web fetch ignored invalid referer header value");
                }
            }
        }
    }

    let response = match req.send().await {
        Ok(r) => r,
        Err(e) => return Ok(WebFetchRunOutcome::Plain(format!("Network Error: {}", e))),
    };

    if !response.status().is_success() {
        let reason = response
            .status()
            .canonical_reason()
            .unwrap_or("Unknown");
        return Ok(WebFetchRunOutcome::Plain(format!(
            "HTTP Error {}: {}",
            response.status().as_u16(),
            reason
        )));
    }

    let html = match response.text().await {
        Ok(t) => t,
        Err(e) => return Ok(WebFetchRunOutcome::Plain(format!("Error reading response body: {}", e))),
    };

    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec![
            "script", "style", "nav", "footer", "noscript", "aside", "form", "svg", "header",
        ])
        .build();
    let markdown = converter
        .convert(&html)
        .unwrap_or_else(|_| "Failed to parse HTML".into());
    let outbound_links = extract_ranked_page_links(&html, &url);
    let sanitized = sanitize_markdown_noise(&markdown);
    let focused = focus_article_text(&sanitized);
    let text_for_chunks = if focused.is_empty() {
        sanitized.as_str()
    } else {
        focused.as_str()
    };
    let (chunks, preview_head) = bound_chunks_and_preview(
        text_for_chunks,
        rt.max_bytes,
        rt.chunk_chars,
        rt.preview_chars,
    );

    if chunks.is_empty() {
        return Ok(WebFetchRunOutcome::Plain(
            "No meaningful content extracted from URL.".to_string(),
        ));
    }

    let artifact = WebArtifact {
        url: url.clone(),
        chunks: chunks.clone(),
        outbound_links: outbound_links.clone(),
    };
    let serialized = serde_json::to_string(&artifact).map_err(FcpError::ParseFault)?;
    let title = format!("web_artifact:{}", uuid::Uuid::new_v4());
    let stored = rt
        .ephemeral
        .insert(
            &title,
            &serialized,
            vec!["web_artifact".to_string(), "external".to_string()],
            rt.artifact_ttl_secs,
        )
        .await?;

    if let Some(semantic) = &rt.semantic {
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            if let Err(e) = semantic
                .upsert_web_chunk(&stored.staged_id, &url, chunk_index, chunk)
                .await
            {
                tracing::warn!(
                    artifact_id = %stored.staged_id,
                    chunk_index,
                    error = %e,
                    "Failed to index web artifact chunk; lexical fallback remains available"
                );
            }
        }
    }

    Ok(WebFetchRunOutcome::Stored(WebFetchStored {
        artifact_id: stored.staged_id,
        url,
        chunk_count: chunks.len(),
        preview_head,
        outbound_links,
    }))
}

pub(crate) fn default_next_step_hint() -> String {
    "Prefer outbound_links for article URLs (artifact_query returns them too). If an article URL returns HTTP 403, retry web:fetch with referer set to that site’s homepage (https://…/).".to_string()
}
