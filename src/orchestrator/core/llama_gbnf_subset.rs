//! Per-turn GBNF subset compilation for llama.cpp (see plan: align grammar with offered tools).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::engine::grammar::{
    compile_fcp_envelope_grammar_dynamic, schema_to_gbnf_rule, ToolGrammarEntry,
};
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;
use crate::tools::Gatekeeper;

const CACHE_KEY_NO_TOOLS: &str = "__fcp_no_tools__";

#[derive(Default)]
pub(crate) struct GbnfSubsetCache {
    inner: Mutex<HashMap<String, Arc<str>>>,
}

impl GbnfSubsetCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Returns cached or freshly compiled GBNF for exactly `tool_names` (sorted internally for the key).
    ///
    /// Empty `tool_names` yields the no-tools envelope (only `tool_calls: []`).
    pub(crate) fn get_or_compile_subset(
        &self,
        gatekeeper: &Gatekeeper,
        tool_names: &[String],
    ) -> Result<Arc<str>> {
        let mut sorted: Vec<String> = tool_names.to_vec();
        sorted.sort();
        let key: String = if sorted.is_empty() {
            CACHE_KEY_NO_TOOLS.to_string()
        } else {
            sorted.join("\x1e")
        };

        let mut guard = self.inner.lock().map_err(|_| {
            FcpError::EngineFault("GBNF subset cache mutex poisoned".to_string())
        })?;

        if let Some(hit) = guard.get(&key) {
            return Ok(Arc::clone(hit));
        }

        let entries: Vec<ToolGrammarEntry> = sorted
            .iter()
            .map(|name| {
                let per_tool_rules = gatekeeper
                    .parameters_root_schema_for(name)
                    .and_then(|schema| schema_to_gbnf_rule(name, &schema))
                    .map(|(_rule_name, rules)| rules);

                ToolGrammarEntry {
                    name: name.clone(),
                    per_tool_rules,
                }
            })
            .collect();

        let compiled = compile_fcp_envelope_grammar_dynamic(&entries);
        let arc: Arc<str> = Arc::from(compiled.into_boxed_str());
        guard.insert(key, arc.clone());
        Ok(arc)
    }
}

/// Same offered list as slim assembly in [`super::step::Orchestrator::step`].
pub(crate) fn slim_offered_tool_names(
    pre_llm_matched_tools: &[String],
    tool_map_offer_cap: usize,
    moltbook_overlay_latched: bool,
    gatekeeper: &Gatekeeper,
    state: &AgentState,
) -> Vec<String> {
    let mut offered: Vec<String> = if pre_llm_matched_tools.is_empty() {
        vec![]
    } else if tool_map_offer_cap == 0 {
        pre_llm_matched_tools.to_vec()
    } else {
        pre_llm_matched_tools
            .iter()
            .take(tool_map_offer_cap)
            .cloned()
            .collect()
    };

    if moltbook_overlay_latched && !offered.is_empty() {
        for name in gatekeeper.allowed_tool_names_with_prefix(state, "moltbook:") {
            if !offered.contains(&name) {
                offered.push(name);
            }
        }
    }

    let needs_web_find = offered.iter().any(|n| n == "web:fetch" || n == "web:search");
    if needs_web_find {
        let find_allowed = gatekeeper
            .allowed_tool_names_with_prefix(state, "web:")
            .into_iter()
            .any(|n| n == "web:find");
        if find_allowed && !offered.iter().any(|n| n == "web:find") {
            offered.push("web:find".to_string());
        }
    }

    if offered.iter().any(|n| n == "doc:read")
        && !offered.iter().any(|n| n == "vault:write")
        && Gatekeeper::state_allows_tool(state, "vault:write")
    {
        offered.push("vault:write".to_string());
    }

    offered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::traits::Tool;
    use async_trait::async_trait;
    use schemars::{schema_for, JsonSchema};
    use serde::Deserialize;

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    struct SystemHealthStub;

    #[async_trait]
    impl Tool for SystemHealthStub {
        fn name(&self) -> &'static str {
            "system:health"
        }

        fn description(&self) -> &'static str {
            "test"
        }

        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schema_for!(EmptyArgs)
        }

        async fn execute(&self, _args: serde_json::Value) -> crate::executive::error::Result<String> {
            Ok("{}".to_string())
        }
    }

    struct ClockNowStub;

    #[async_trait]
    impl Tool for ClockNowStub {
        fn name(&self) -> &'static str {
            "clock:now"
        }

        fn description(&self) -> &'static str {
            "test"
        }

        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schema_for!(EmptyArgs)
        }

        async fn execute(&self, _args: serde_json::Value) -> crate::executive::error::Result<String> {
            Ok("{}".to_string())
        }
    }

    #[test]
    fn subset_grammar_lists_only_offered_tool_in_alternation() {
        let mut gk = Gatekeeper::new();
        gk.register(Arc::new(SystemHealthStub));
        gk.register(Arc::new(ClockNowStub));

        let full_entries: Vec<ToolGrammarEntry> = gk
            .registered_tool_names()
            .iter()
            .map(|name| {
                let per_tool_rules = gk
                    .parameters_root_schema_for(name)
                    .and_then(|schema| schema_to_gbnf_rule(name, &schema))
                    .map(|(_n, rules)| rules);
                ToolGrammarEntry {
                    name: name.clone(),
                    per_tool_rules,
                }
            })
            .collect();
        let full_grammar = compile_fcp_envelope_grammar_dynamic(&full_entries);

        let cache = GbnfSubsetCache::new();
        let subset = cache
            .get_or_compile_subset(&gk, &[String::from("system:health")])
            .expect("subset compile");

        assert!(
            subset.len() < full_grammar.len(),
            "subset grammar should be smaller than full-registry grammar"
        );
        assert!(
            subset.contains("system:health"),
            "subset should mention the offered tool"
        );
        assert!(
            !subset.contains("clock:now"),
            "subset must not include a tool omitted from the offered set"
        );
    }

    #[test]
    fn slim_offered_matches_step_union_when_moltbook_latched() {
        let mut gk = Gatekeeper::new();
        gk.register(Arc::new(SystemHealthStub));
        let pre = vec!["system:health".to_string()];
        let out = slim_offered_tool_names(&pre, 10, true, &gk, &AgentState::Chat);
        assert_eq!(out, vec!["system:health".to_string()]);
    }

    #[test]
    fn slim_offered_pairs_web_find_with_fetch() {
        use crate::tools::web::{WebFetchTool, WebFindTool, WebSearchTool};
        use crate::tools::web::context::{WebFetcherKind, WebToolContext};
        use crate::tools::web::WebSessionLedger;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let mut gk = Gatekeeper::new();
        let ctx = WebToolContext {
            vault_root: std::env::temp_dir(),
            web: crate::config::WebConfig::default(),
            web_fetch_user_agent: "test".into(),
            num_ctx: 8192,
            vault_read_ratio: 0.5,
            web_fetch_chunk_chars: 7_372,
            web_fetch_max_bytes: 20480,
            web_allowlist_override: None,
            ledger: Arc::new(Mutex::new(WebSessionLedger::new())),
            fetcher: WebFetcherKind::Browser39 {
                binary: "browser39".into(),
            },
        };
        gk.register(Arc::new(WebFetchTool { ctx: ctx.clone() }));
        gk.register(Arc::new(WebFindTool {
            ctx: ctx.clone(),
            max_snippet_chars: 600,
            max_total_chars: 2000,
        }));
        gk.register(Arc::new(WebSearchTool { ctx }));
        let pre = vec!["web:fetch".to_string()];
        let out = slim_offered_tool_names(&pre, 10, false, &gk, &AgentState::Chat);
        assert!(out.contains(&"web:fetch".to_string()));
        assert!(out.contains(&"web:find".to_string()));
    }
}
