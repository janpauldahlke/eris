//! Vault mission cache pipeline for `web:fetch` and `news:today` internal fetches.

use crate::executive::error::{FcpError, Result};
use crate::ingest::{bound_chunks_and_preview, truncate_char_boundary};
use crate::tools::web::allowlist::{enforce_allowlist, load_allowlist};
use crate::tools::web::budget::WebBudget;
use crate::tools::web::cache::{WebMissionStore, WebPageRecord};
use crate::tools::web::consent::{
    accept_texts_for_host, fetch_with_consent_blocking, load_consent_profiles, ConsentOutcome,
};
use crate::tools::web::context::{WebFetcherKind, WebToolContext};
use crate::tools::web::fetcher::{FetchedPage, WebFetcher};
use crate::tools::web::ledger::{host_from_normalized_url, WebCacheHit};
use crate::tools::web::links::{
    absolutize_outbound_links, filter_same_host_links, is_news_today_homepage_mission,
    rank_headline_links, rank_internal_links_with_cap, HEADLINE_LINK_CAP, INTERNAL_LINK_CAP,
};
use crate::tools::web::artifact::WebOutboundLink;
use crate::vault_layout;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(crate) const FALLBACK_WEB_FETCH_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Max outbound links embedded in the tool receipt JSON (full list stays in vault `links.json`).
pub const RECEIPT_INTERNAL_LINK_SAMPLE: usize = 5;

/// Max chars for `preview_head` in the tool receipt (vault chunks keep the full body).
pub const RECEIPT_PREVIEW_MAX_CHARS: usize = 600;

/// Heuristic page body quality after noise stripping (additive receipt field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageQuality {
    Ok,
    Thin,
    LikelyConsentOrJs,
    /// Teaser / subscription wall (e.g. ZEIT Z+) — body present but not a full article.
    LikelyPaywall,
}

impl PageQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Thin => "thin",
            Self::LikelyConsentOrJs => "likely_consent_or_js",
            Self::LikelyPaywall => "likely_paywall",
        }
    }
}

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

/// LLM-facing fetch receipt — field order is intentional (diagnostics before large blobs).
#[derive(Serialize)]
pub struct WebFetchReceiptJson {
    pub receipt_summary: String,
    pub page_quality: String,
    pub consent_attempted: bool,
    pub consent_improved: bool,
    pub next_step_hint: String,
    pub artifact_id: String,
    pub mission_id: String,
    pub url: String,
    pub normalized_url: String,
    pub chunk_count: usize,
    pub fetch_budget_remaining: u32,
    #[serde(default)]
    pub cached: bool,
    pub internal_link_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub internal_links_sample: Vec<WebOutboundLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sitemap_hint: Option<String>,
    pub preview_head: String,
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
                    receipt_json: serde_json::to_string(&cached_receipt(
                        &hit,
                        &args,
                        &ctx.vault_root,
                        ctx.web.thin_page_char_threshold,
                    ))
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
    let host = host_from_normalized_url(&reservation.normalized_url).unwrap_or_default();
    let fetcher = ctx.fetcher_for_host(&host, &artifact_id);
    let (page, consent_outcome) = fetch_paginated(
        ctx,
        &host,
        fetcher.as_ref(),
        &url,
        selector.as_deref(),
        &budget,
    )
    .await?;

    let sanitized = sanitize_markdown_noise(&page.markdown);
    let outbound_link_count = page.links.len();
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
    let internal_links = if is_news_today_homepage_mission(args.mission_note.as_deref()) {
        rank_headline_links(
            outbound,
            &args.url,
            args.mission_note.as_deref(),
            HEADLINE_LINK_CAP,
        )
    } else {
        rank_internal_links_with_cap(outbound, args.mission_note.as_deref(), INTERNAL_LINK_CAP)
    };
    let thin_threshold = ctx.web.thin_page_char_threshold;
    let page_quality = classify_page_quality(&sanitized, outbound_link_count, thin_threshold);
    let is_serp = args
        .mission_note
        .as_deref()
        .is_some_and(|n| n.trim().starts_with("web:search:"));

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

    let (consent_attempted, consent_improved) = consent_bools(consent_outcome);
    let next_step_hint = build_next_step_hint(
        &artifact_id,
        reservation.budget_remaining_after,
        page_quality,
        is_serp,
        consent_outcome,
        thin_threshold,
    );
    let receipt = build_web_fetch_receipt(WebFetchReceiptParts {
        artifact_id: artifact_id.clone(),
        mission_id: reservation.mission_id,
        url: args.url,
        normalized_url,
        chunk_count: chunks.len(),
        preview_head,
        internal_links,
        sitemap_hint: if args.explore_site {
            Some("explore_site enabled — map only when user asked".into())
        } else {
            None
        },
        fetch_budget_remaining: reservation.budget_remaining_after,
        cached: false,
        page_quality,
        consent_attempted,
        consent_improved,
        next_step_hint,
    });
    Ok(WebFetchRunOutcome::Stored(WebFetchStored {
        receipt_json: serde_json::to_string(&receipt).map_err(FcpError::ParseFault)?,
    }))
}

async fn fetch_paginated(
    ctx: &WebToolContext,
    host: &str,
    fetcher: &dyn WebFetcher,
    url: &str,
    selector: Option<&str>,
    budget: &WebBudget,
) -> Result<(FetchedPage, Option<ConsentOutcome>)> {
    let mut offset = 0u32;
    let mut combined = String::new();
    let mut links = Vec::new();
    let mut truncated;
    let mut next_offset;
    let mut consent_outcome = None;

    loop {
        let page = if offset == 0 {
            let (page, consent) =
                fetch_page_with_optional_consent(ctx, host, fetcher, url, selector, budget.page_max_tokens, offset)
                    .await?;
            consent_outcome = consent;
            page
        } else {
            fetcher
                .fetch_page(url, selector, budget.page_max_tokens, offset)
                .await?
        };
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

    Ok((
        FetchedPage {
            markdown: combined,
            links,
            truncated,
            next_offset,
        },
        consent_outcome,
    ))
}

async fn fetch_page_with_optional_consent(
    ctx: &WebToolContext,
    host: &str,
    fetcher: &dyn WebFetcher,
    url: &str,
    selector: Option<&str>,
    max_tokens: u32,
    offset: u32,
) -> Result<(FetchedPage, Option<ConsentOutcome>)> {
    if !ctx.web.consent_helper_enabled || ctx.web.use_legacy_batch {
        let page = fetcher.fetch_page(url, selector, max_tokens, offset).await?;
        return Ok((page, None));
    }
    let WebFetcherKind::Browser39 { binary } = &ctx.fetcher else {
        let page = fetcher.fetch_page(url, selector, max_tokens, offset).await?;
        return Ok((page, None));
    };
    let profiles = load_consent_profiles(&ctx.vault_root)?;
    let accept_texts = accept_texts_for_host(&profiles, host);
    let session_dir = ctx.browser39_session_dir(host, "consent");
    let config_path = ctx.vault_root.join(".fcp/browser39/config.toml");
    let persist = ctx.effective_persist_browser39();
    let binary = binary.clone();
    let url = url.to_string();
    let selector = selector.map(str::to_string);
    let thin_threshold = ctx.web.thin_page_char_threshold;
    let max_attempts = ctx.web.consent_max_attempts;
    let (page, outcome) = tokio::task::spawn_blocking(move || {
        fetch_with_consent_blocking(
            &binary,
            &config_path,
            &session_dir,
            persist,
            &url,
            selector.as_deref(),
            max_tokens,
            offset,
            &accept_texts,
            max_attempts,
            thin_threshold,
        )
    })
    .await
    .map_err(|e| FcpError::ToolFault {
        tool_name: "web:fetch".into(),
        reason: format!("consent fetch task join failed: {e}"),
    })??;
    Ok((page, Some(outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_thin_page_after_sanitize() {
        let body = "x".repeat(150);
        let q = classify_page_quality(&body, 2, 300);
        assert_eq!(q, PageQuality::Thin);
    }

    #[test]
    fn classify_likely_consent_when_nearly_empty() {
        let q = classify_page_quality("ok", 0, 300);
        assert_eq!(q, PageQuality::LikelyConsentOrJs);
    }

    #[test]
    fn classify_likely_paywall_on_teaser_markers() {
        let body = "Headline here.\n\nJetzt abonnieren für Z+ Inhalte. Nur für Abonnenten.";
        let q = classify_page_quality(body, 5, 300);
        assert_eq!(q, PageQuality::LikelyPaywall);
    }

    #[test]
    fn serp_hint_mentions_web_find_not_refetch() {
        let hint = build_next_step_hint("art-serp", 1, PageQuality::Ok, true, None, 300);
        assert!(hint.contains("web:find"));
        assert!(hint.contains("Do not web:fetch"));
    }

    #[test]
    fn receipt_json_puts_diagnostics_before_preview() {
        let receipt = build_web_fetch_receipt(WebFetchReceiptParts {
            artifact_id: "art-1".into(),
            mission_id: "mis-1".into(),
            url: "https://example.com".into(),
            normalized_url: "https://example.com/".into(),
            chunk_count: 2,
            preview_head: "z".repeat(800),
            internal_links: Vec::new(),
            sitemap_hint: None,
            fetch_budget_remaining: 1,
            cached: false,
            page_quality: PageQuality::Ok,
            consent_attempted: false,
            consent_improved: false,
            next_step_hint: "hint".into(),
        });
        let json = serde_json::to_string(&receipt).expect("json");
        let pq = json.find("\"page_quality\"").expect("page_quality");
        let preview = json.find("\"preview_head\"").expect("preview_head");
        assert!(pq < preview, "diagnostics must precede preview_head");
        assert!(json.contains("\"consent_attempted\":false"));
        assert!(json.contains("receipt_summary"));
        assert!(receipt.preview_head.chars().count() <= RECEIPT_PREVIEW_MAX_CHARS + 1);
    }

    #[test]
    fn consent_bools_default_false_when_helper_disabled() {
        let (a, i) = consent_bools(None);
        assert!(!a && !i);
    }

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

fn cached_receipt(
    hit: &WebCacheHit,
    args: &WebFetchArgs,
    vault_root: &Path,
    thin_threshold: usize,
) -> WebFetchReceiptJson {
    let store = WebMissionStore::new(vault_root);
    let mut chunk_count = 0usize;
    let mut preview_head = String::new();
    let mut page_quality = PageQuality::Ok;
    let mut fetch_budget_remaining = 0u32;

    if let Ok(meta) = store.read_page_meta(&hit.mission_id, &hit.artifact_id) {
        chunk_count = meta.chunk_count as usize;
        if let Ok(indices) = store.list_chunk_indices(&hit.mission_id, &hit.artifact_id) {
            let mut combined = String::new();
            for idx in indices.iter().take(8) {
                if let Ok(chunk) = store.read_chunk(&hit.mission_id, &hit.artifact_id, *idx) {
                    if !preview_head.is_empty() {
                        preview_head.push('\n');
                    }
                    preview_head.push_str(&chunk);
                    combined.push_str(&chunk);
                    combined.push('\n');
                }
            }
            let link_count = store
                .read_links(&hit.mission_id, &hit.artifact_id)
                .map(|v| v.len())
                .unwrap_or(0);
            page_quality = classify_page_quality(&combined, link_count, thin_threshold);
        }
    }
    if let Ok(manifest) = store.load_manifest(&hit.mission_id) {
        fetch_budget_remaining = manifest.budget_remaining();
    }

    let next_step_hint = format!(
        "Duplicate URL (cache hit) — do not treat as a new fetch. Use web:find on artifact_id `{}` \
         (chunk_count={chunk_count}, fetch_budget_remaining={fetch_budget_remaining}). {}",
        hit.artifact_id,
        vault_mission_hint()
    );
    build_web_fetch_receipt(WebFetchReceiptParts {
        artifact_id: hit.artifact_id.clone(),
        mission_id: hit.mission_id.clone(),
        url: args.url.clone(),
        normalized_url: hit.normalized_url.clone(),
        chunk_count,
        preview_head,
        internal_links: Vec::new(),
        sitemap_hint: None,
        fetch_budget_remaining,
        cached: true,
        page_quality,
        consent_attempted: false,
        consent_improved: false,
        next_step_hint,
    })
}

struct WebFetchReceiptParts {
    artifact_id: String,
    mission_id: String,
    url: String,
    normalized_url: String,
    chunk_count: usize,
    preview_head: String,
    internal_links: Vec<WebOutboundLink>,
    sitemap_hint: Option<String>,
    fetch_budget_remaining: u32,
    cached: bool,
    page_quality: PageQuality,
    consent_attempted: bool,
    consent_improved: bool,
    next_step_hint: String,
}

fn consent_bools(consent: Option<ConsentOutcome>) -> (bool, bool) {
    match consent {
        Some(c) => (c.attempted, c.improved),
        None => (false, false),
    }
}

fn build_receipt_summary(
    page_quality: PageQuality,
    consent_attempted: bool,
    consent_improved: bool,
    chunk_count: usize,
    artifact_id: &str,
    cached: bool,
) -> String {
    format!(
        "page_quality={} consent_attempted={} consent_improved={} chunk_count={} cached={} artifact_id={}",
        page_quality.as_str(),
        consent_attempted,
        consent_improved,
        chunk_count,
        cached,
        artifact_id
    )
}

fn build_web_fetch_receipt(parts: WebFetchReceiptParts) -> WebFetchReceiptJson {
    let link_count = parts.internal_links.len();
    let sample: Vec<WebOutboundLink> = parts
        .internal_links
        .into_iter()
        .take(RECEIPT_INTERNAL_LINK_SAMPLE)
        .collect();
    let receipt_summary = build_receipt_summary(
        parts.page_quality,
        parts.consent_attempted,
        parts.consent_improved,
        parts.chunk_count,
        &parts.artifact_id,
        parts.cached,
    );
    WebFetchReceiptJson {
        receipt_summary,
        page_quality: parts.page_quality.as_str().to_string(),
        consent_attempted: parts.consent_attempted,
        consent_improved: parts.consent_improved,
        next_step_hint: parts.next_step_hint,
        artifact_id: parts.artifact_id,
        mission_id: parts.mission_id,
        url: parts.url,
        normalized_url: parts.normalized_url,
        chunk_count: parts.chunk_count,
        fetch_budget_remaining: parts.fetch_budget_remaining,
        cached: parts.cached,
        internal_link_count: link_count,
        internal_links_sample: sample,
        sitemap_hint: parts.sitemap_hint,
        preview_head: truncate_char_boundary(&parts.preview_head, RECEIPT_PREVIEW_MAX_CHARS),
    }
}

/// Heuristic markers for subscription / plus-article walls (German and English news sites).
pub(crate) fn markdown_suggests_paywall(markdown: &str) -> bool {
    let lower = markdown.to_lowercase();
    const MARKERS: &[&str] = &[
        "z+ inhalte",
        "z+ artikel",
        "plus-artikel",
        "jetzt abonnieren",
        "nur für abonnenten",
        "für abonnenten",
        "abo abschließen",
        "diesen artikel kaufen",
        "registrieren um fortzufahren",
        "paywall",
        "exklusiv für",
        "mit z+ lesen",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

pub(crate) fn classify_page_quality(
    markdown: &str,
    link_count: usize,
    thin_threshold: usize,
) -> PageQuality {
    let chars = markdown.trim().chars().count();
    if markdown_suggests_paywall(markdown) && chars < thin_threshold.saturating_mul(3) {
        return PageQuality::LikelyPaywall;
    }
    if chars >= thin_threshold {
        return PageQuality::Ok;
    }
    if chars < 80 && link_count == 0 {
        return PageQuality::LikelyConsentOrJs;
    }
    if chars < thin_threshold {
        return PageQuality::Thin;
    }
    PageQuality::Ok
}

fn vault_mission_hint() -> &'static str {
    "Full page text is under `20_Discourse/web/missions/<mission_id>/pages/<artifact_id>/` — use web:find with artifact_id, not vault:read on web_artifacts paths."
}

fn build_next_step_hint(
    artifact_id: &str,
    budget_remaining: u32,
    page_quality: PageQuality,
    is_serp: bool,
    consent: Option<ConsentOutcome>,
    thin_threshold: usize,
) -> String {
    if is_serp {
        return format!(
            "SERP cached — use web:find on artifact_id `{artifact_id}` with your search terms. \
             Do not web:fetch this search-results URL again. {}",
            vault_mission_hint()
        );
    }
    if let Some(c) = consent {
        if c.attempted && !c.improved && page_quality != PageQuality::Ok {
            let click_note = if c.all_clicks_failed {
                "browser39 could not click any accept label in static HTML (CMP may need JS). \
                 Add `[[cookies]]` for this host in `.fcp/browser39/config.toml` (export from a real browser) \
                 or tune `.fcp/browser39/consent_profiles.toml`. "
            } else {
                "Consent clicks ran but the page stayed thin. "
            };
            return format!(
                "{click_note}Page still below {thin_threshold} chars of markdown. \
                 Use web:find on artifact_id `{artifact_id}`. {}",
                vault_mission_hint()
            );
        }
        if c.attempted && c.improved {
            return format!(
                "Consent accept-link clicks improved the fetch — use web:find on artifact_id `{artifact_id}`. {}",
                vault_mission_hint()
            );
        }
    }
    match page_quality {
        PageQuality::Ok => find_first_hint(artifact_id, budget_remaining),
        PageQuality::Thin => format!(
            "Page body is thin after fetch (cookie banners stripped or little HTML). \
             Try web:find on artifact_id `{artifact_id}`; consent profiles may need tuning for this host. {}",
            vault_mission_hint()
        ),
        PageQuality::LikelyConsentOrJs => format!(
            "Very little text extracted — likely consent wall or JS-only content (browser39 does not run site JavaScript). \
             Use web:find on artifact_id `{artifact_id}` for whatever was stored; for full pages add `[[cookies]]` in \
             `.fcp/browser39/config.toml` or accept labels in consent_profiles.toml. {}",
            vault_mission_hint()
        ),
        PageQuality::LikelyPaywall => format!(
            "Likely paywall or plus-article teaser (subscription markers in body). browser39 only sees the public HTML. \
             Use web:find on artifact_id `{artifact_id}` for the teaser, or fetch an open site (e.g. taz.de) for full depth. \
             Do not claim you read the full article. {}",
            vault_mission_hint()
        ),
    }
}

fn find_first_hint(artifact_id: &str, budget_remaining: u32) -> String {
    format!(
        "Use web:find on artifact_id `{artifact_id}` with mission terms before another web:fetch. \
         fetch_budget_remaining={budget_remaining}. No site-wide BFS unless explore_site is enabled. {}",
        vault_mission_hint()
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
