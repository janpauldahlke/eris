//! Lexical search over vault mission page chunks (`web:find`).

use crate::executive::error::{FcpError, Result};
use crate::ingest::trim_chars;
use crate::tools::context_view_hint::{ARTIFACT_QUERY_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::tools::web::cache::WebMissionStore;
use crate::tools::web::context::WebToolContext;
use async_trait::async_trait;
use schemars::JsonSchema;
use schemars::schema::RootSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, JsonSchema)]
pub struct WebFindArgs {
    pub artifact_id: String,
    pub query: String,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub mission_id: Option<String>,
    #[serde(default)]
    pub mission_note: Option<String>,
}

#[derive(Serialize)]
struct FindMatch {
    chunk_index: u32,
    score: f32,
    snippet: String,
}

#[derive(Serialize)]
struct WebFindResponse {
    artifact_id: String,
    mission_id: String,
    url: String,
    matches: Vec<FindMatch>,
    #[serde(default)]
    suggest_stop: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggest_stop_reason: Option<String>,
}

pub struct WebFindTool {
    pub ctx: WebToolContext,
    pub max_snippet_chars: usize,
    pub max_total_chars: usize,
}

fn tokenize(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn score_chunk(chunk: &str, tokens: &[String]) -> f32 {
    let c = chunk.to_lowercase();
    tokens.iter().map(|t| usize::from(c.contains(t))).sum::<usize>() as f32
}

fn chunk_heading_weight(chunk: &str) -> f32 {
    if chunk.lines().any(|l| l.starts_with("# ")) {
        1.15
    } else if chunk.lines().any(|l| l.starts_with("## ")) {
        1.08
    } else {
        1.0
    }
}

fn suggest_stop_heuristic(
    matches: &[FindMatch],
    mission_note: Option<&str>,
) -> (bool, Option<String>) {
    let Some(note) = mission_note.filter(|n| !n.trim().is_empty()) else {
        if matches.first().is_some_and(|m| m.score >= 2.0) {
            return (
                true,
                Some("Strong lexical match in fetched page.".into()),
            );
        }
        return (false, None);
    };
    let note_tokens = tokenize(note);
    if note_tokens.is_empty() {
        return (false, None);
    }
    let Some(top) = matches.first() else {
        return (false, None);
    };
    let snippet = top.snippet.to_lowercase();
    let hits = note_tokens
        .iter()
        .filter(|t| snippet.contains(t.as_str()))
        .count();
    if hits >= 2 || (hits >= 1 && top.score >= 2.0) {
        (
            true,
            Some("Mission note terms appear in top snippet — you may answer the user.".into()),
        )
    } else {
        (false, None)
    }
}

#[async_trait]
impl Tool for WebFindTool {
    fn name(&self) -> &'static str {
        "web:find"
    }

    fn description(&self) -> &'static str {
        "Lexical search within a fetched web page's vault chunks (by artifact_id). Use after web:fetch before fetching the same host again."
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: ARTIFACT_QUERY_SNIPPET_CHARS,
        }
    }

    fn parameters_schema(&self) -> RootSchema {
        schemars::schema_for!(WebFindArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: WebFindArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let top_k = args.top_k.unwrap_or(5).clamp(1, 12);

        let mission_id = resolve_mission_id(&self.ctx, &args).await?;
        let store = WebMissionStore::new(&self.ctx.vault_root);
        let meta = store.read_page_meta(&mission_id, &args.artifact_id)?;
        let indices = store.list_chunk_indices(&mission_id, &args.artifact_id)?;
        let tokens = tokenize(&args.query);
        let mut scored: Vec<(u32, f32, String)> = Vec::new();
        for idx in indices {
            let chunk = store.read_chunk(&mission_id, &args.artifact_id, idx)?;
            let score = score_chunk(&chunk, &tokens) * chunk_heading_weight(&chunk);
            if score > 0.0 {
                scored.push((
                    idx,
                    score,
                    trim_chars(&chunk, self.max_snippet_chars),
                ));
            }
        }
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let matches: Vec<FindMatch> = scored
            .into_iter()
            .take(top_k)
            .map(|(chunk_index, score, snippet)| FindMatch {
                chunk_index,
                score,
                snippet,
            })
            .collect();

        {
            let mut ledger = self.ctx.ledger.lock().await;
            ledger.record_find(&args.artifact_id);
            let _ = ledger.save_to_vault(&self.ctx.vault_root, &self.ctx.web);
        }

        let (suggest_stop, suggest_stop_reason) =
            suggest_stop_heuristic(&matches, args.mission_note.as_deref());

        let body = WebFindResponse {
            artifact_id: args.artifact_id,
            mission_id: mission_id.clone(),
            url: meta.url,
            matches,
            suggest_stop,
            suggest_stop_reason,
        };
        let mut json = serde_json::to_string(&body).map_err(FcpError::ParseFault)?;
        if json.len() > self.max_total_chars {
            let notice = "\n[web:find: output truncated]";
            let cap = self.max_total_chars.saturating_sub(notice.len());
            json = format!("{}{}", trim_chars(&json, cap), notice);
        }
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebConfig;
    use crate::tools::web::cache::WebMissionStore;
    use crate::tools::web::context::WebFetcherKind;
    use crate::tools::web::fetcher::MockWebFetcher;
    use crate::tools::web::fetch_inner::run_vault_web_fetch;
    use crate::tools::web::fetch_inner::WebFetchArgs;
    use crate::tools::web::WebSessionLedger;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    #[tokio::test]
    async fn find_returns_lexical_hit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let allowlist_path = dir.path().join(".fcp/web_allowlist.toml");
        std::fs::create_dir_all(allowlist_path.parent().expect("p")).expect("mkdir");
        std::fs::write(&allowlist_path, r#"patterns = ["https://example.com/**"]"#).expect("w");
        let ctx = WebToolContext {
            vault_root: dir.path().to_path_buf(),
            web: WebConfig::default(),
            web_fetch_user_agent: "test".into(),
            num_ctx: 8192,
            vault_read_ratio: 0.5,
            web_fetch_max_bytes: 20480,
            web_allowlist_override: None,
            ledger: Arc::new(Mutex::new(WebSessionLedger::new())),
            fetcher: WebFetcherKind::Mock(Arc::new(MockWebFetcher::example_com())),
        };
        let fetch_out = run_vault_web_fetch(
            &ctx,
            WebFetchArgs {
                url: "https://example.com/page".into(),
                mission_note: Some("Product X price".into()),
                mission_id: None,
                fetch_budget: Some(2),
                selector: None,
                explore_site: false,
            },
        )
        .await
        .expect("fetch");
        let receipt = match fetch_out {
            crate::tools::web::fetch_inner::WebFetchRunOutcome::Stored(s) => {
                serde_json::from_str::<serde_json::Value>(&s.receipt_json).expect("receipt")
            }
            _ => panic!("expected stored"),
        };
        let mission_id = receipt["mission_id"].as_str().expect("mid").to_string();
        let artifact_id = receipt["artifact_id"].as_str().expect("aid").to_string();
        let _store = WebMissionStore::new(dir.path());
        let tool = WebFindTool {
            ctx,
            max_snippet_chars: 400,
            max_total_chars: 2000,
        };
        let out = tool
            .execute(serde_json::json!({
                "artifact_id": artifact_id,
                "query": "Product X",
                "mission_id": mission_id,
                "mission_note": "Product X price"
            }))
            .await
            .expect("find");
        assert!(out.contains("Product X"));
        assert!(out.contains("suggest_stop"));
    }
}

async fn resolve_mission_id(ctx: &WebToolContext, args: &WebFindArgs) -> Result<String> {
    if let Some(mid) = args.mission_id.as_deref().filter(|s| !s.trim().is_empty()) {
        return Ok(mid.trim().to_string());
    }
    let ledger = ctx.ledger.lock().await;
    if let Some(mid) = ledger.mission_id_for_artifact(&args.artifact_id) {
        return Ok(mid);
    }
    Err(FcpError::SchemaViolation(
        "web:find requires mission_id when artifact is unknown to session ledger".into(),
    ))
}
