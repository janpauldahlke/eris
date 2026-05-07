//! `news:today` — homepage fetch plus curated outbound headlines and optional deep article fetches in one tool call.

use crate::executive::error::{FcpError, Result};
use crate::ingest::truncate_char_boundary;
use crate::memory::ephemeral::EphemeralMemory;
use crate::memory::semantic::SemanticBrain;
use crate::tools::context_view_hint::{ARTIFACT_QUERY_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::tools::web::artifact::WebOutboundLink;
use crate::tools::web::fetch_inner::{
    WebFetchRunOutcome, WebFetchRuntime, build_web_fetch_client, default_next_step_hint,
    run_web_fetch,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use url::Url;

/// Relative paths under [`NewsTodayConfigSnapshot::site_base`] for [`NewsTodayArgs::category`].
/// `homepage_url` always wins if set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NewsTodayCategory {
    /// International — path `news/world`
    World,
    /// UK — path `news/uk`
    Uk,
    /// Path `news/politics`
    Politics,
    /// Business and economy — path `news/business`
    #[serde(alias = "economics")]
    Business,
    /// Science, climate, nature — path `news/science_and_environment`
    Science,
    /// Path `news/technology`
    #[serde(alias = "tech")]
    Technology,
    /// Path `sport` (top-level site section)
    #[serde(alias = "sports")]
    Sport,
    /// Path `health`
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
    if path.is_empty() {
        return Err(FcpError::SchemaViolation(
            "news:today: internal empty category path".into(),
        ));
    }
    base.join(path).map(|u| u.to_string()).map_err(|_| {
        FcpError::SchemaViolation(format!(
            "news:today: could not join path {path:?} to site base {raw}"
        ))
    })
}

/// Snapshot of [`crate::config::AppConfig`] fields needed by [`NewsTodayTool`].
#[derive(Clone)]
pub struct NewsTodayConfigSnapshot {
    /// Origin for [`NewsTodayArgs::category`] (e.g. `https://www.bbc.com`).
    pub site_base: String,
    pub default_homepage: Option<String>,
    pub max_headlines_default: usize,
    pub deep_fetch_max_default: u8,
    pub allowed_hosts: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct NewsTodayArgs {
    /// Site section (world, politics, business, science, …): resolved as `{news_today_site_base}/{path}` (see tool docs). Prefer when the user names a topic. Ignored if `homepage_url` is set.
    #[serde(default)]
    pub category: Option<NewsTodayCategory>,
    /// Fully qualified homepage URL (`https://…`). Overrides `category`. Omit if config sets `news_today_default_homepage`.
    #[serde(default)]
    pub homepage_url: Option<String>,
    /// Max headline rows from ranked outbound links (capped at 20).
    #[serde(default)]
    pub max_headlines: Option<usize>,
    /// Fetch full text for the first N ranked article URLs after the homepage (max 3).
    #[serde(default)]
    pub deep_fetch_top_n: Option<u8>,
    /// Optional HTTP Referer for homepage and article requests (some origins require homepage referer).
    #[serde(default)]
    pub referer: Option<String>,
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
    homepage_url: String,
    homepage_artifact_id: String,
    headlines: Vec<HeadlineRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deep_articles: Vec<DeepArticleRow>,
    hint: String,
}

pub struct NewsTodayTool {
    rt: WebFetchRuntime,
    snapshot: NewsTodayConfigSnapshot,
}

impl NewsTodayTool {
    pub fn new(
        timeout_secs: u64,
        max_bytes: usize,
        chunk_chars: usize,
        preview_chars: usize,
        artifact_ttl_secs: u64,
        user_agent: String,
        default_referer: Option<String>,
        ephemeral: Arc<EphemeralMemory>,
        semantic: Option<Arc<SemanticBrain>>,
        snapshot: NewsTodayConfigSnapshot,
    ) -> Self {
        let client = build_web_fetch_client(timeout_secs, &user_agent);
        Self {
            rt: WebFetchRuntime {
                client,
                max_bytes,
                chunk_chars,
                preview_chars,
                artifact_ttl_secs,
                default_referer,
                ephemeral,
                semantic,
            },
            snapshot,
        }
    }
}

fn resolve_homepage(args: &NewsTodayArgs, snapshot: &NewsTodayConfigSnapshot) -> Result<String> {
    let from_arg = args
        .homepage_url
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if let Some(u) = from_arg {
        return Ok(u);
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
                "news:today requires homepage_url, category, or config news_today_default_homepage (default https://www.bbc.com/)"
                    .into(),
            )
        })
}

fn host_allowed(target: &Url, homepage: &Url, cfg_hosts: &[String]) -> bool {
    let Some(t_host) = target.host_str() else {
        return false;
    };
    let Some(h_home) = homepage.host_str() else {
        return false;
    };
    if cfg_hosts.is_empty() {
        return t_host.eq_ignore_ascii_case(h_home);
    }
    t_host.eq_ignore_ascii_case(h_home)
        || cfg_hosts
            .iter()
            .any(|h| h.trim().eq_ignore_ascii_case(t_host))
}

fn normalize_same_document(a: &Url, b: &Url) -> bool {
    let ma = a.clone();
    let mb = b.clone();
    ma.scheme() == mb.scheme() && ma.host_str() == mb.host_str() && ma.path() == mb.path()
}

/// Ranked outbound links filtered by host policy; skips the homepage document itself.
fn curated_headlines(
    links: &[WebOutboundLink],
    homepage: &Url,
    max: usize,
    cfg_hosts: &[String],
) -> Vec<WebOutboundLink> {
    let mut out: Vec<WebOutboundLink> = Vec::new();
    for link in links {
        let Ok(u) = Url::parse(&link.url) else {
            continue;
        };
        if normalize_same_document(&u, homepage) {
            continue;
        }
        if !host_allowed(&u, homepage, cfg_hosts) {
            continue;
        }
        let scheme = u.scheme();
        if scheme != "http" && scheme != "https" {
            continue;
        }
        out.push(link.clone());
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
        "Fetch a news homepage once, return ranked headline links (titles + URLs), optionally deep-fetch top articles in the same call—avoids duplicate-suppressed repeated web:fetch with identical args in one turn. Prefer over chained web:fetch for headline browsing. Pass category (e.g. politics, science, business, sport) for BBC section fronts instead of the generic hub when the user names a topic."
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(NewsTodayArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: ARTIFACT_QUERY_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: NewsTodayArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let homepage_str = resolve_homepage(&args, &self.snapshot)?;

        if !homepage_str.starts_with("http://") && !homepage_str.starts_with("https://") {
            return Err(FcpError::SchemaViolation(
                "homepage URL must start with http:// or https://".into(),
            ));
        }

        let homepage_url = Url::parse(&homepage_str)
            .map_err(|_| FcpError::SchemaViolation("news:today: invalid homepage URL".into()))?;

        let max_headlines = args
            .max_headlines
            .unwrap_or(self.snapshot.max_headlines_default)
            .clamp(1, 20);

        let deep_n = args
            .deep_fetch_top_n
            .unwrap_or(self.snapshot.deep_fetch_max_default)
            .min(3);

        let referer_arg = args.referer.clone();

        let homepage_result =
            run_web_fetch(&self.rt, homepage_str.clone(), referer_arg.clone()).await?;

        let stored = match homepage_result {
            WebFetchRunOutcome::Plain(msg) => {
                return Ok(msg);
            }
            WebFetchRunOutcome::Stored(s) => s,
        };

        let headlines_raw = curated_headlines(
            &stored.outbound_links,
            &homepage_url,
            max_headlines,
            &self.snapshot.allowed_hosts,
        );

        let headlines: Vec<HeadlineRow> = headlines_raw
            .iter()
            .map(|l| HeadlineRow {
                rank: l.rank,
                title: l.anchor_text.clone(),
                url: l.url.clone(),
            })
            .collect();

        let mut deep_articles: Vec<DeepArticleRow> = Vec::new();
        if deep_n > 0 {
            let mut seen: HashSet<String> = HashSet::new();
            seen.insert(homepage_str.clone());

            for link in headlines_raw.iter().take(deep_n as usize) {
                if seen.contains(&link.url) {
                    continue;
                }
                seen.insert(link.url.clone());

                let article_result = run_web_fetch(
                    &self.rt,
                    link.url.clone(),
                    Some(referer_arg.clone().unwrap_or_else(|| homepage_str.clone())),
                )
                .await?;

                match article_result {
                    WebFetchRunOutcome::Plain(_) => {}
                    WebFetchRunOutcome::Stored(a) => {
                        let preview = truncate_char_boundary(&a.preview_head, 480);
                        deep_articles.push(DeepArticleRow {
                            url: a.url,
                            artifact_id: a.artifact_id,
                            preview_head: preview,
                            chunk_count: a.chunk_count,
                        });
                    }
                }
            }
        }

        let response = NewsTodayResponse {
            homepage_url: homepage_str,
            homepage_artifact_id: stored.artifact_id,
            headlines,
            deep_articles,
            hint: format!(
                "{} Use web:artifact_query with homepage_artifact_id or deep article artifact_ids for more text. Prefer news:today over repeating identical web:fetch in one turn (duplicate calls are suppressed).",
                default_next_step_hint()
            ),
        };

        serde_json::to_string(&response).map_err(FcpError::ParseFault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ephemeral::EphemeralMemory;
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn news_today_returns_headlines_from_homepage() {
        let server = MockServer::start().await;
        let html = r#"<!DOCTYPE html><html><body>
<a href="/news/article-one">Long headline about something important with enough anchor text here</a>
</body></html>"#;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(html))
            .mount(&server)
            .await;

        let ephemeral = Arc::new(EphemeralMemory::new("news_test_ws".into()));
        let tool = NewsTodayTool::new(
            5,
            20480,
            1024,
            256,
            60,
            crate::tools::web::fetch_inner::FALLBACK_WEB_FETCH_UA.to_string(),
            None,
            ephemeral,
            None,
            NewsTodayConfigSnapshot {
                site_base: "https://www.bbc.com".into(),
                default_homepage: None,
                max_headlines_default: 12,
                deep_fetch_max_default: 0,
                allowed_hosts: vec![],
            },
        );

        let base = server.uri();
        let hp = format!("{}/", base.trim_end_matches('/'));
        let args = json!({ "homepage_url": hp });

        let out = tool.execute(args).await.expect("execute");
        let v: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert!(v["homepage_artifact_id"].as_str().is_some());
        let headlines = v["headlines"].as_array().expect("headlines array");
        assert!(!headlines.is_empty());
        assert!(
            headlines[0]["url"]
                .as_str()
                .unwrap_or("")
                .contains("/news/article-one")
        );
    }

    #[test]
    fn host_allowed_same_host_only() {
        let h = Url::parse("https://www.bbc.com/").unwrap();
        let ok = Url::parse("https://www.bbc.com/news/foo").unwrap();
        let bad = Url::parse("https://evil.com/x").unwrap();
        assert!(host_allowed(&ok, &h, &[]));
        assert!(!host_allowed(&bad, &h, &[]));
    }

    #[test]
    fn host_allowed_extra_config_hosts() {
        let h = Url::parse("https://www.bbc.com/").unwrap();
        let sister = Url::parse("https://bbc.co.uk/foo").unwrap();
        assert!(!host_allowed(&sister, &h, &[]));
        assert!(host_allowed(&sister, &h, &["bbc.co.uk".to_string()]));
    }

    #[test]
    fn bbc_category_joins_site_base() {
        use NewsTodayCategory::*;
        let base = "https://www.bbc.com";
        for cat in [
            World, Uk, Politics, Business, Science, Technology, Sport, Health,
        ] {
            let s = super::join_site_base(base, super::bbc_category_path(cat)).expect("join");
            assert!(s.starts_with("https://"), "category {cat:?} url {s}");
            Url::parse(&s).expect("parse category url");
        }
        assert_eq!(
            super::join_site_base(base, "news/politics").expect("politics"),
            "https://www.bbc.com/news/politics"
        );
    }

    #[test]
    fn news_today_args_category_aliases() {
        let j = json!({ "category": "economics" });
        let a: NewsTodayArgs = serde_json::from_value(j).expect("economics alias");
        assert_eq!(a.category, Some(NewsTodayCategory::Business));

        let j = json!({ "category": "tech" });
        let a: NewsTodayArgs = serde_json::from_value(j).expect("tech alias");
        assert_eq!(a.category, Some(NewsTodayCategory::Technology));

        let j = json!({ "category": "sports" });
        let a: NewsTodayArgs = serde_json::from_value(j).expect("sports alias");
        assert_eq!(a.category, Some(NewsTodayCategory::Sport));
    }

    #[test]
    fn homepage_url_overrides_category() {
        let snapshot = NewsTodayConfigSnapshot {
            site_base: "https://www.bbc.com".into(),
            default_homepage: Some("https://www.bbc.com/news".into()),
            max_headlines_default: 12,
            deep_fetch_max_default: 0,
            allowed_hosts: vec![],
        };
        let args: NewsTodayArgs = serde_json::from_value(json!({
            "category": "sport",
            "homepage_url": "https://example.com/"
        }))
        .unwrap();
        assert_eq!(
            super::resolve_homepage(&args, &snapshot).expect("resolve"),
            "https://example.com/"
        );
    }

    #[test]
    fn resolve_category_uses_site_base() {
        let snapshot = NewsTodayConfigSnapshot {
            site_base: "https://www.bbc.com".into(),
            default_homepage: Some("https://www.bbc.com/".into()),
            max_headlines_default: 12,
            deep_fetch_max_default: 0,
            allowed_hosts: vec![],
        };
        let args: NewsTodayArgs = serde_json::from_value(json!({ "category": "science" })).unwrap();
        assert_eq!(
            super::resolve_homepage(&args, &snapshot).expect("resolve"),
            "https://www.bbc.com/news/science_and_environment"
        );
    }
}
