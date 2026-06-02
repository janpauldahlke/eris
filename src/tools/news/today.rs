//! `news:today` — homepage fetch plus curated outbound headlines and optional deep article fetches.

use crate::executive::error::{FcpError, Result};
use crate::ingest::truncate_char_boundary;
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use crate::tools::web::allowlist::{enforce_allowlist, load_allowlist};
use crate::tools::web::artifact::WebOutboundLink;
use crate::tools::web::cache::WebMissionStore;
use crate::tools::web::context::WebToolContext;
use crate::tools::web::fetch_inner::{
    WebFetchArgs, WebFetchRunOutcome, parse_stored_receipt, run_vault_web_fetch,
    run_vault_web_fetch_simple,
};
use crate::tools::web::ledger::{host_from_normalized_url, normalize_url};
use crate::tools::web::links::{filter_headline_candidates, select_deep_fetch_links};
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use url::Url;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NewsTodayCategory {
    World,
    Uk,
    Politics,
    #[serde(alias = "economics")]
    Business,
    Science,
    #[serde(alias = "tech")]
    Technology,
    #[serde(alias = "sports")]
    Sport,
    #[serde(alias = "wellbeing")]
    Health,
}

fn bbc_category_path(cat: NewsTodayCategory) -> &'static str {
    match cat {
        NewsTodayCategory::World => "news/world",
        NewsTodayCategory::Uk => "news/uk",
        NewsTodayCategory::Politics => "news/politics",
        NewsTodayCategory::Business => "news/business",
        NewsTodayCategory::Science => "news/science_and_environment",
        NewsTodayCategory::Technology => "news/technology",
        NewsTodayCategory::Sport => "sport",
        NewsTodayCategory::Health => "health",
    }
}

fn join_site_base(site_base: &str, rel_path: &str) -> Result<String> {
    let raw = site_base.trim();
    if raw.is_empty() {
        return Err(FcpError::SchemaViolation(
            "news:today: news_today_site_base is empty".into(),
        ));
    }
    let base = Url::parse(raw).map_err(|_| {
        FcpError::SchemaViolation(format!("news:today: invalid news_today_site_base ({raw})"))
    })?;
    let path = rel_path.trim().trim_start_matches('/');
    base.join(path)
        .map(|u| u.to_string())
        .map_err(|_| FcpError::SchemaViolation("news:today: could not join category path".into()))
}

#[derive(Clone)]
pub struct NewsTodayConfigSnapshot {
    pub site_base: String,
    pub default_homepage: Option<String>,
    pub max_headlines_default: usize,
    pub deep_fetch_max_default: u8,
}

#[derive(Deserialize, JsonSchema)]
pub struct NewsTodayArgs {
    #[serde(default)]
    pub category: Option<NewsTodayCategory>,
    #[serde(default)]
    pub homepage_url: Option<String>,
    #[serde(default)]
    pub max_headlines: Option<usize>,
    #[serde(default)]
    pub deep_fetch_top_n: Option<u8>,
}

#[derive(Serialize)]
struct HeadlineRow {
    rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    url: String,
}

#[derive(Serialize)]
struct DeepArticleRow {
    url: String,
    artifact_id: String,
    preview_head: String,
    chunk_count: usize,
}

#[derive(Serialize)]
struct NewsTodayResponse {
    receipt_summary: String,
    homepage_url: String,
    homepage_artifact_id: String,
    mission_id: String,
    headline_count: usize,
    deep_fetch_count: usize,
    /// Article URLs selected for deep fetch (from ranked headlines).
    deep_fetch_urls: Vec<String>,
    headlines: Vec<HeadlineRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deep_articles: Vec<DeepArticleRow>,
    hint: String,
}

pub struct NewsTodayTool {
    pub ctx: WebToolContext,
    pub snapshot: NewsTodayConfigSnapshot,
}

impl NewsTodayTool {
    pub fn new(ctx: WebToolContext, snapshot: NewsTodayConfigSnapshot) -> Self {
        Self { ctx, snapshot }
    }
}

fn resolve_homepage(args: &NewsTodayArgs, snapshot: &NewsTodayConfigSnapshot) -> Result<String> {
    if let Some(u) = args
        .homepage_url
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        return Ok(u.to_string());
    }
    if let Some(cat) = args.category {
        return join_site_base(&snapshot.site_base, bbc_category_path(cat));
    }
    snapshot
        .default_homepage
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            FcpError::SchemaViolation(
                "news:today requires homepage_url, category, or config news_today_default_homepage"
                    .into(),
            )
        })
}

fn clamp_deep_fetch_top_n(ctx: &WebToolContext, requested: u8) -> u8 {
    let ledger = ctx.ledger.try_lock();
    let Ok(ledger) = ledger else {
        return requested.min(3);
    };
    let turn_room = ctx
        .web
        .max_fetches_per_user_turn
        .saturating_sub(ledger.fetches_this_turn())
        .saturating_sub(1);
    let session_room = ctx
        .web
        .max_fetches_per_chat_session
        .saturating_sub(ledger.fetches_this_session())
        .saturating_sub(1);
    requested.min(3).min(turn_room as u8).min(session_room as u8)
}

fn curated_headlines(links: &[WebOutboundLink], homepage: &Url, max: usize) -> Vec<WebOutboundLink> {
    let homepage_str = homepage.as_str();
    let filtered = filter_headline_candidates(links.to_vec(), homepage_str);
    let mut out = Vec::new();
    for link in filtered {
        let Ok(u) = Url::parse(&link.url) else {
            continue;
        };
        if u.scheme() != "http" && u.scheme() != "https" {
            continue;
        }
        if u.path() == homepage.path() && u.host_str() == homepage.host_str() {
            continue;
        }
        out.push(link);
        if out.len() >= max {
            break;
        }
    }
    out
}

#[async_trait]
impl Tool for NewsTodayTool {
    fn name(&self) -> &'static str {
        "news:today"
    }

    fn description(&self) -> &'static str {
        "Fetch a news homepage and optional top article bodies. `homepage_url` may be any allowlisted site (BBC, taz.de, etc.); omit it to use config default or pass `category` for BBC section paths. URLs must match `.fcp/web_allowlist.toml`. Uses one tool-call budget unit."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(NewsTodayArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Full
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: NewsTodayArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let homepage_str = resolve_homepage(&args, &self.snapshot)?;
        let allowlist = load_allowlist(
            &self.ctx.vault_root,
            self.ctx.web_allowlist_override.as_deref(),
        )?;
        enforce_allowlist(self.ctx.web.allowlist_enabled, &homepage_str, &allowlist)?;

        let homepage_url = Url::parse(&homepage_str)
            .map_err(|_| FcpError::SchemaViolation("news:today: invalid homepage URL".into()))?;
        let max_headlines = args
            .max_headlines
            .unwrap_or(self.snapshot.max_headlines_default)
            .clamp(1, 20);
        let deep_n = clamp_deep_fetch_top_n(
            &self.ctx,
            args.deep_fetch_top_n
                .unwrap_or(self.snapshot.deep_fetch_max_default),
        );

        // Let the ledger allocate mission_id on first fetch (passing a fresh UUID as Some(...)
        // fails reserve_fetch because the mission is not registered yet).
        let homepage_outcome = run_vault_web_fetch(
            &self.ctx,
            WebFetchArgs {
                url: homepage_str.clone(),
                mission_note: Some("news:today homepage".into()),
                mission_id: None,
                fetch_budget: Some(1 + deep_n as u32),
                selector: None,
                explore_site: false,
                ledger_dedup_preserves_query: false,
            },
        )
        .await?;

        let (homepage_artifact_id, mission_id) = match homepage_outcome {
            WebFetchRunOutcome::Plain(msg) => return Ok(msg),
            WebFetchRunOutcome::Stored(s) => parse_stored_receipt(&s.receipt_json)?,
        };

        let store = WebMissionStore::new(&self.ctx.vault_root);
        let links = store
            .read_links(&mission_id, &homepage_artifact_id)
            .unwrap_or_default();
        let headlines_raw = curated_headlines(&links, &homepage_url, max_headlines);
        let headlines: Vec<HeadlineRow> = headlines_raw
            .iter()
            .map(|l| HeadlineRow {
                rank: l.rank,
                title: l.anchor_text.clone(),
                url: l.url.clone(),
            })
            .collect();

        let deep_candidates =
            select_deep_fetch_links(&links, &homepage_str, deep_n as usize);
        let mut deep_articles = Vec::new();
        let mut seen = HashSet::new();
        seen.insert(homepage_str.clone());
        for link in deep_candidates.iter().take(deep_n as usize) {
            if !seen.insert(link.url.clone()) {
                continue;
            }
            enforce_allowlist(self.ctx.web.allowlist_enabled, &link.url, &allowlist)?;
            let article_outcome =
                run_vault_web_fetch_simple(&self.ctx, link.url.clone(), &mission_id).await?;
            if let WebFetchRunOutcome::Stored(s) = article_outcome {
                let v: serde_json::Value = serde_json::from_str(&s.receipt_json)?;
                let artifact_id = v
                    .get("artifact_id")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string();
                let preview = v
                    .get("preview_head")
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                let chunk_count = v.get("chunk_count").and_then(|c| c.as_u64()).unwrap_or(0) as usize;
                deep_articles.push(DeepArticleRow {
                    url: link.url.clone(),
                    artifact_id,
                    preview_head: truncate_char_boundary(preview, 480),
                    chunk_count,
                });
            }
        }

        if let Ok(normalized) = normalize_url(&homepage_str) {
            if let Some(host) = host_from_normalized_url(&normalized) {
                let mut ledger = self.ctx.ledger.lock().await;
                ledger.clear_host_pending_find(&host);
                let _ = ledger.save_to_vault(&self.ctx.vault_root, &self.ctx.web);
            }
        }

        let deep_fetch_urls: Vec<String> = deep_articles.iter().map(|d| d.url.clone()).collect();
        let headline_count = headlines.len();
        let deep_fetch_count = deep_articles.len();
        let receipt_summary = format!(
            "headline_count={headline_count} deep_fetch_count={deep_fetch_count} homepage_artifact_id={homepage_artifact_id} mission_id={mission_id}"
        );
        let hint = if headlines.is_empty() {
            "Homepage was fetched but no headline links were extracted (often relative links on the page). Try web:find on homepage_artifact_id, or web:fetch a section URL.".into()
        } else {
            format!(
                "Use web:find with homepage_artifact_id `{homepage_artifact_id}` for homepage text. \
                 Deep article bodies are in deep_articles[] (artifact_id per row). \
                 After news:today you may web:fetch the same homepage host again without web:find first."
            )
        };
        let response = NewsTodayResponse {
            receipt_summary,
            homepage_url: homepage_str,
            homepage_artifact_id,
            mission_id,
            headline_count,
            deep_fetch_count,
            deep_fetch_urls,
            headlines,
            deep_articles,
            hint,
        };
        serde_json::to_string(&response).map_err(FcpError::ParseFault)
    }
}
