//! Vault mission cache pipeline for `web:fetch` and `news:today` internal fetches.

use crate::executive::error::{FcpError, Result};
use crate::ingest::bound_chunks_and_preview;
use crate::tools::web::allowlist::{enforce_allowlist, load_allowlist};
use crate::tools::web::budget::WebBudget;
use crate::tools::web::cache::{WebMissionStore, WebPageRecord};
use crate::tools::web::context::WebToolContext;
use crate::tools::web::fetcher::{FetchedPage, WebFetcher};
use crate::tools::web::ledger::{host_from_normalized_url, WebCacheHit};
use crate::tools::web::links::{
    absolutize_outbound_links, filter_same_host_links, rank_internal_links,
};
use crate::tools::web::artifact::WebOutboundLink;
use crate::vault_layout;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) const FALLBACK_WEB_FETCH_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

#[derive(Deserialize, Serialize, schemars::JsonSchema)]
pub struct WebFetchArgs {
    pub url: String,
    #[serde(default)]
    pub mission_note: Option<String>,
    #[serde(default)]
    pub mission_id: Option<String>,
    #[serde(default)]
    pub fetch_budget: Option<u32>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub explore_site: bool,
}

#[derive(Serialize)]
pub struct WebFetchReceiptJson {
    pub artifact_id: String,
    pub mission_id: String,
    pub url: String,
    pub normalized_url: String,
    pub chunk_count: usize,
    pub preview_head: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal_links: Option<Vec<WebOutboundLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sitemap_hint: Option<String>,
    pub fetch_budget_remaining: u32,
    #[serde(default)]
    pub cached: bool,
    pub next_step_hint: String,
}

pub struct WebFetchStored {
    pub receipt_json: String,
}

pub enum WebFetchRunOutcome {
    Stored(WebFetchStored),
    Plain(String),
}

/// Parse `artifact_id` and `mission_id` from a stored fetch receipt JSON blob.
pub fn parse_stored_receipt(receipt_json: &str) -> Result<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(receipt_json).map_err(FcpError::ParseFault)?;
    let artifact_id = v
        .get("artifact_id")
        .and_then(|a| a.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mission_id = v
        .get("mission_id")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if artifact_id.is_empty() || mission_id.is_empty() {
        return Err(FcpError::ToolFault {
            tool_name: "web:fetch".into(),
            reason: "stored receipt missing artifact_id or mission_id".into(),
        });
    }
    Ok((artifact_id, mission_id))
}

/// Internal fetch used by `news:today` (allowlist + ledger + vault; no mission_note on receipt).
pub async fn run_vault_web_fetch_simple(
    ctx: &WebToolContext,
    url: String,
    mission_id: &str,
) -> Result<WebFetchRunOutcome> {
    run_vault_web_fetch(
        ctx,
        WebFetchArgs {
            url,
            mission_note: None,
            mission_id: Some(mission_id.to_string()),
            fetch_budget: None,
            selector: None,
            explore_site: false,
        },
    )
    .await
}

pub async fn run_vault_web_fetch(
    ctx: &WebToolContext,
    args: WebFetchArgs,
) -> Result<WebFetchRunOutcome> {
    if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
        return Err(FcpError::SchemaViolation(
            "URL must start with http:// or https://".into(),
        ));
    }
    if args.explore_site && !ctx.web.explore_site_enabled {
        return Err(FcpError::PolicyViolation {
            code: "WEB_EXPLORE_SITE_DISABLED".to_string(),
            message: "explore_site requires web.explore_site_enabled = true in config".into(),
        });
    }

    let allowlist = load_allowlist(
        &ctx.vault_root,
        ctx.web_allowlist_override.as_deref(),
    )?;
    enforce_allowlist(ctx.web.allowlist_enabled, &args.url, &allowlist)?;

    let budget = WebBudget::from_parts(
        ctx.num_ctx,
        ctx.vault_read_ratio,
        ctx.web_fetch_max_bytes,
    );

    let artifact_id = Uuid::new_v4().to_string();
    let new_mission_id = Uuid::new_v4().to_string();

    let reservation = {
        let mut ledger = ctx.ledger.lock().await;
        match ledger.reserve_fetch(
            &ctx.web,
            &args.url,
            args.mission_id.as_deref(),
            args.fetch_budget,
            &artifact_id,
            &new_mission_id,
        )? {
            Ok(res) => res,
            Err(hit) => {
                return Ok(WebFetchRunOutcome::Stored(WebFetchStored {
                    receipt_json: serde_json::to_string(&cached_receipt(&hit, &args))
                        .map_err(FcpError::ParseFault)?,
                }));
            }
        }
    };

    let store = WebMissionStore::new(&ctx.vault_root);
    if store.load_manifest(&reservation.mission_id).is_err() {
        store.create_mission(
            &reservation.mission_id,
            args.mission_note.as_deref(),
            reservation.budget_max,
        )?;
    }

    ensure_browser39_config(&ctx.vault_root, &ctx.web_fetch_user_agent)?;

    let url = args.url.clone();
    let selector = args.selector.clone();
    let fetcher = ctx.fetcher_for_artifact(&artifact_id);
    let page = fetch_paginated(
        fetcher.as_ref(),
        &url,
        selector.as_deref(),
        &budget,
    )
    .await?;

    let sanitized = sanitize_markdown_noise(&page.markdown);
    let (chunks, preview_head) = bound_chunks_and_preview(
        &sanitized,
        budget.max_bytes,
        budget.chunk_chars,
        budget.preview_chars,
    );
    if chunks.is_empty() {
        return Ok(WebFetchRunOutcome::Plain(
            "No meaningful content extracted from URL.".into(),
        ));
    }

    let host = host_from_normalized_url(&reservation.normalized_url).unwrap_or_default();
    let page_record = WebPageRecord {
        url: args.url.clone(),
        normalized_url: reservation.normalized_url.clone(),
        fetched_at: Utc::now(),
        host: host.clone(),
        truncated: page.truncated,
        chunk_count: chunks.len() as u32,
    };

    let mut outbound = absolutize_outbound_links(page.links, &args.url);
    if !args.explore_site {
        outbound = filter_same_host_links(outbound, &args.url);
    }
    let internal_links = rank_internal_links(outbound, args.mission_note.as_deref());

    store.write_page(
        &reservation.mission_id,
        &artifact_id,
        &page_record,
        &chunks,
        &internal_links,
    )?;
    store.record_page_fetch(
        &reservation.mission_id,
        &args.url,
        &reservation.normalized_url,
        &artifact_id,
    )?;

    let normalized_url = reservation.normalized_url.clone();
    {
        let mut ledger = ctx.ledger.lock().await;
        ledger.commit_fetch(
            normalized_url.clone(),
            artifact_id.clone(),
            reservation.mission_id.clone(),
            host,
        );
        let _ = ledger.save_to_vault(&ctx.vault_root, &ctx.web);
    }

    let receipt = WebFetchReceiptJson {
        artifact_id: artifact_id.clone(),
        mission_id: reservation.mission_id,
        url: args.url,
        normalized_url,
        chunk_count: chunks.len(),
        preview_head,
        internal_links: Some(internal_links),
        sitemap_hint: if args.explore_site {
            Some("explore_site enabled — map only when user asked".into())
        } else {
            None
        },
        fetch_budget_remaining: reservation.budget_remaining_after,
        cached: false,
        next_step_hint: find_first_hint(&artifact_id, reservation.budget_remaining_after),
    };
    Ok(WebFetchRunOutcome::Stored(WebFetchStored {
        receipt_json: serde_json::to_string(&receipt).map_err(FcpError::ParseFault)?,
    }))
}

async fn fetch_paginated(
    fetcher: &dyn WebFetcher,
    url: &str,
    selector: Option<&str>,
    budget: &WebBudget,
) -> Result<FetchedPage> {
    let mut offset = 0u32;
    let mut combined = String::new();
    let mut links = Vec::new();
    let mut truncated;
    let mut next_offset;

    loop {
        let page = fetcher
            .fetch_page(url, selector, budget.page_max_tokens, offset)
            .await?;
        if combined.is_empty() {
            links = page.links;
        }
        combined.push_str(&page.markdown);
        truncated = page.truncated;
        next_offset = page.next_offset;
        if combined.len() >= budget.max_bytes {
            break;
        }
        if !page.truncated {
            break;
        }
        let Some(next) = page.next_offset else {
            break;
        };
        offset = next;
        if offset > budget.max_chunks as u32 * budget.page_max_tokens {
            break;
        }
    }

    Ok(FetchedPage {
        markdown: combined,
        links,
        truncated,
        next_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stored_receipt_reads_ids() {
        let json = serde_json::json!({
            "artifact_id": "art-1",
            "mission_id": "mis-1",
            "url": "https://example.com",
            "normalized_url": "https://example.com",
            "chunk_count": 2,
            "preview_head": "hi",
            "fetch_budget_remaining": 1,
            "cached": false,
            "next_step_hint": "find"
        });
        let (a, m) = parse_stored_receipt(&json.to_string()).expect("parse");
        assert_eq!(a, "art-1");
        assert_eq!(m, "mis-1");
    }
}

fn cached_receipt(hit: &WebCacheHit, args: &WebFetchArgs) -> WebFetchReceiptJson {
    WebFetchReceiptJson {
        artifact_id: hit.artifact_id.clone(),
        mission_id: hit.mission_id.clone(),
        url: args.url.clone(),
        normalized_url: hit.normalized_url.clone(),
        chunk_count: 0,
        preview_head: String::new(),
        internal_links: None,
        sitemap_hint: None,
        fetch_budget_remaining: 0,
        cached: true,
        next_step_hint: format!(
            "Cached URL — use web:find on artifact_id `{}` before fetching again.",
            hit.artifact_id
        ),
    }
}

fn find_first_hint(artifact_id: &str, budget_remaining: u32) -> String {
    format!(
        "Use web:find on artifact_id `{artifact_id}` with mission terms before another web:fetch. fetch_budget_remaining={budget_remaining}. No site-wide BFS unless explore_site is enabled."
    )
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

fn ensure_browser39_config(vault_root: &Path, user_agent: &str) -> Result<PathBuf> {
    let path = vault_layout::fcp_dir(vault_root).join("browser39/config.toml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(FcpError::Io)?;
    }
    let ua = user_agent.trim();
    let ua = if ua.is_empty() {
        FALLBACK_WEB_FETCH_UA
    } else {
        ua
    };
    let body = if path.is_file() {
        let existing = std::fs::read_to_string(&path).map_err(FcpError::Io)?;
        merge_user_agent_toml(&existing, ua)
    } else {
        format!(
            "# browser39 config (eris-managed user_agent)\n[user_agent]\nvalue = \"{ua}\"\n"
        )
    };
    std::fs::write(&path, body).map_err(FcpError::Io)?;
    Ok(path)
}

fn merge_user_agent_toml(existing: &str, user_agent: &str) -> String {
    let escaped = user_agent.replace('\\', "\\\\").replace('\"', "\\\"");
    if existing.contains("[user_agent]") {
        let mut out = String::new();
        let mut in_ua = false;
        let mut replaced = false;
        for line in existing.lines() {
            if line.trim() == "[user_agent]" {
                in_ua = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_ua && line.trim_start().starts_with("value") {
                out.push_str(&format!("value = \"{escaped}\"\n"));
                replaced = true;
                in_ua = false;
                continue;
            }
            if in_ua && line.starts_with('[') {
                in_ua = false;
            }
            out.push_str(line);
            out.push('\n');
        }
        if !replaced {
            out.push_str(&format!("\n[user_agent]\nvalue = \"{escaped}\"\n"));
        }
        out
    } else {
        format!("{existing}\n[user_agent]\nvalue = \"{escaped}\"\n")
    }
}
