//! Pluggable page fetch backend (`browser39` subprocess or mock).

use crate::executive::error::{FcpError, Result};
use tracing::{debug, info, warn};
use crate::tools::web::artifact::WebOutboundLink;
use async_trait::async_trait;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub markdown: String,
    pub links: Vec<WebOutboundLink>,
    pub truncated: bool,
    pub next_offset: Option<u32>,
}

#[async_trait]
pub trait WebFetcher: Send + Sync {
    async fn fetch_page(
        &self,
        url: &str,
        selector: Option<&str>,
        max_tokens: u32,
        offset: u32,
    ) -> Result<FetchedPage>;
}

#[derive(Debug, Clone, Deserialize)]
struct Browser39LinkRow {
    #[serde(default)]
    i: u32,
    text: String,
    href: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Browser39FetchResult {
    markdown: String,
    #[serde(default)]
    links: Vec<Browser39LinkRow>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    next_offset: Option<u32>,
}

pub struct MockWebFetcher {
    pub markdown: String,
    pub links: Vec<WebOutboundLink>,
}

impl MockWebFetcher {
    pub fn example_com() -> Self {
        Self {
            markdown: "# Example\n\nProduct X costs $42.\n".into(),
            links: vec![WebOutboundLink {
                url: "https://example.com/next".into(),
                anchor_text: Some("Next".into()),
                rank: 1,
            }],
        }
    }
}

#[async_trait]
impl WebFetcher for MockWebFetcher {
    async fn fetch_page(
        &self,
        _url: &str,
        _selector: Option<&str>,
        _max_tokens: u32,
        _offset: u32,
    ) -> Result<FetchedPage> {
        Ok(FetchedPage {
            markdown: self.markdown.clone(),
            links: self.links.clone(),
            truncated: false,
            next_offset: None,
        })
    }
}

pub struct Browser39Fetcher {
    pub binary: String,
    pub config_path: std::path::PathBuf,
    pub session_dir: std::path::PathBuf,
    /// When true, pass `--no-persist` (isolated batch). Host consent flows need `false`.
    pub no_persist: bool,
}

#[async_trait]
impl WebFetcher for Browser39Fetcher {
    async fn fetch_page(
        &self,
        url: &str,
        selector: Option<&str>,
        max_tokens: u32,
        offset: u32,
    ) -> Result<FetchedPage> {
        let session_dir = self.session_dir.clone();
        let binary = self.binary.clone();
        let config_path = self.config_path.clone();
        let url = url.to_string();
        let selector = selector.map(str::to_string);
        let no_persist = self.no_persist;
        tokio::task::spawn_blocking(move || {
            browser39_fetch_blocking(
                &binary,
                &config_path,
                &session_dir,
                no_persist,
                &url,
                selector.as_deref(),
                max_tokens,
                offset,
            )
        })
        .await
        .map_err(|e| FcpError::ToolFault {
            tool_name: "web:fetch".into(),
            reason: format!("browser39 task join failed: {e}"),
        })?
    }
}

fn browser39_line_preview(line: &str, max_chars: usize) -> String {
    line.chars().take(max_chars).collect()
}

/// Parse one `results.jsonl` line from browser39 1.7.x (`batch` flattens payload; older lines may nest `result`).
pub(crate) fn parse_browser39_fetch_line(line: &str) -> Result<FetchedPage> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(FcpError::ParseFault)?;
    if value.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        let reason = value
            .get("error")
            .and_then(|e| e.as_str())
            .or_else(|| value.get("message").and_then(|m| m.as_str()))
            .unwrap_or("browser39 fetch failed");
        warn!(
            event = "web.browser39.fetch_error",
            error = %reason,
            preview = %browser39_line_preview(line, 400),
            "browser39 reported ok=false"
        );
        return Err(FcpError::ToolFault {
            tool_name: "web:fetch".into(),
            reason: reason.to_string(),
        });
    }
    let payload = match value.get("result") {
        Some(r) => r.clone(),
        None => value,
    };
    let parsed: Browser39FetchResult = serde_json::from_value(payload).map_err(|e| {
        warn!(
            event = "web.browser39.parse_error",
            error = %e,
            preview = %browser39_line_preview(line, 400),
            "browser39 result JSON did not match expected fetch shape"
        );
        FcpError::ToolFault {
            tool_name: "web:fetch".into(),
            reason: format!("browser39 result parse: {e}"),
        }
    })?;
    let links = parsed
        .links
        .into_iter()
        .map(|row| WebOutboundLink {
            url: row.href,
            anchor_text: Some(row.text),
            rank: row.i.max(1).min(u32::MAX as u32).max(1),
        })
        .collect::<Vec<_>>();
    debug!(
        event = "web.browser39.fetch_ok",
        markdown_chars = parsed.markdown.len(),
        link_count = links.len(),
        truncated = parsed.truncated,
        next_offset = ?parsed.next_offset,
        "browser39 fetch parsed"
    );
    Ok(FetchedPage {
        markdown: parsed.markdown,
        links,
        truncated: parsed.truncated,
        next_offset: parsed.next_offset,
    })
}

/// Run one or more JSONL commands in a shared browser39 batch session.
pub(crate) fn browser39_run_batch_blocking(
    binary: &str,
    config_path: &std::path::Path,
    session_dir: &std::path::Path,
    no_persist: bool,
    commands: &[serde_json::Value],
) -> Result<Vec<String>> {
    std::fs::create_dir_all(session_dir).map_err(FcpError::Io)?;
    let input = session_dir.join("commands.jsonl");
    let output = session_dir.join("results.jsonl");

    let body = commands
        .iter()
        .map(|cmd| serde_json::to_string(cmd).map_err(FcpError::ParseFault))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    let body = if body.is_empty() {
        String::new()
    } else {
        format!("{body}\n")
    };

    info!(
        event = "web.browser39.batch_start",
        command_count = commands.len(),
        binary = %binary,
        config = %config_path.display(),
        session_dir = %session_dir.display(),
        no_persist,
        "spawning browser39 batch"
    );

    std::fs::write(&input, body).map_err(FcpError::Io)?;
    let _ = std::fs::remove_file(&output);

    let mut cmd = std::process::Command::new(binary);
    cmd.arg("batch")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .arg("--config")
        .arg(config_path);
    if no_persist {
        cmd.arg("--no-persist");
    }
    let output_proc = cmd.output().map_err(FcpError::Io)?;
    if !output_proc.status.success() {
        let stderr = String::from_utf8_lossy(&output_proc.stderr);
        warn!(
            event = "web.browser39.exit_error",
            exit = ?output_proc.status,
            stderr = %stderr.chars().take(800).collect::<String>(),
            "browser39 batch process failed"
        );
        return Err(FcpError::ToolFault {
            tool_name: "web:fetch".into(),
            reason: format!("browser39 exited with {}", output_proc.status),
        });
    }

    let raw = std::fs::read_to_string(&output).map_err(FcpError::Io)?;
    Ok(raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

fn browser39_fetch_blocking(
    binary: &str,
    config_path: &std::path::Path,
    session_dir: &std::path::Path,
    no_persist: bool,
    url: &str,
    selector: Option<&str>,
    max_tokens: u32,
    offset: u32,
) -> Result<FetchedPage> {
    let mut options = serde_json::json!({
        "max_tokens": max_tokens,
        "offset": offset,
        "include_links": true,
        "strip_nav": true,
        "show_selectors_first": false,
    });
    if let Some(sel) = selector.filter(|s| !s.trim().is_empty()) {
        options["selector"] = serde_json::Value::String(sel.to_string());
    }
    let cmd = serde_json::json!({
        "id": "fetch-1",
        "action": "fetch",
        "v": 1,
        "seq": 1,
        "url": url,
        "options": options,
    });
    let lines = browser39_run_batch_blocking(
        binary,
        config_path,
        session_dir,
        no_persist,
        &[cmd],
    )?;
    parse_first_batch_fetch_line(&lines, url)
}

fn parse_first_batch_fetch_line(lines: &[String], url: &str) -> Result<FetchedPage> {
    lines
        .iter()
        .find_map(|line| parse_browser39_fetch_line(line).ok())
        .ok_or_else(|| {
            warn!(
                event = "web.browser39.empty_output",
                url = %url,
                "browser39 produced no results lines"
            );
            FcpError::ToolFault {
                tool_name: "web:fetch".into(),
                reason: "browser39 produced no results".into(),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_browser39_flat_batch_line() {
        let line = r##"{"id":"fetch-1","ok":true,"seq":1,"url":"https://example.com","markdown":"# Hi","links":[{"i":1,"text":"Next","href":"https://example.com/n"}],"truncated":false}"##;
        let page = parse_browser39_fetch_line(line).expect("parse");
        assert!(page.markdown.contains("Hi"));
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].url, "https://example.com/n");
    }

    #[test]
    fn parse_browser39_nested_result_line() {
        let line = r#"{"ok":true,"result":{"markdown":"nested","links":[],"truncated":true,"next_offset":100}}"#;
        let page = parse_browser39_fetch_line(line).expect("parse");
        assert_eq!(page.markdown, "nested");
        assert!(page.truncated);
        assert_eq!(page.next_offset, Some(100));
    }
}
