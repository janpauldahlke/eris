use crate::engine::LlmEngine;
use crate::orchestrator::tool_router::ToolRouter;
use crate::presentation::SYSTEM_ALARM_PREFIX;
use crate::telemetry::routing_codes;
use std::time::Instant;

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    /// Conversational vs tool mode, plus ordered router names for Tier 1 (Top-K).
    pub(super) async fn run_pre_llm_routing(&mut self) -> (bool, Vec<String>) {
        let user_input = self.last_user_content();
        let turn_seq = self.turn_seq;

        if user_input.starts_with(SYSTEM_ALARM_PREFIX) {
            let alarm_payload = user_input
                .strip_prefix(SYSTEM_ALARM_PREFIX)
                .unwrap_or(user_input);
            if alarm_payload.to_ascii_lowercase().contains("moltbook") {
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_ALARM_TOOL_ELIGIBLE,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                    turn_seq,
                    tools_needed = true,
                    router_match_count = 0usize,
                    "system alarm prefix with Moltbook label; semantic tool routing enabled"
                );
            } else {
                self.last_router_ms = 0;
                self.last_top_tool_match = None;
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_CONV_ALARM,
                    outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                    turn_seq,
                    tools_needed = false,
                    router_match_count = 0usize,
                    "system alarm prefix; conversational mode"
                );
                return (false, Vec::new());
            }
        }

        if ToolRouter::short_input_guard_conversational_only(user_input) {
            self.last_router_ms = 0;
            self.last_top_tool_match = None;
            tracing::info!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_PRELLM_CONV_SHORT_INPUT,
                outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                turn_seq,
                tools_needed = false,
                router_match_count = 0usize,
                "short-input guard; conversational mode"
            );
            return (false, Vec::new());
        }

        let Some(router) = &self.tool_router else {
            self.last_router_ms = 0;
            self.last_top_tool_match = None;
            tracing::warn!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_PRELLM_ROUTER_UNAVAILABLE,
                outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                turn_seq,
                tools_needed = true,
                router_match_count = 0usize,
                "no tool router; roster-only tool mode"
            );
            return (true, Vec::new());
        };

        let router_started = Instant::now();
        match router.match_tools(user_input).await {
            Ok(matches) if matches.is_empty() => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = None;
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_SEMANTIC_EMPTY,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                    turn_seq,
                    tools_needed = true,
                    router_match_count = 0usize,
                    "no semantic tool match; tool fallback mode"
                );
                (true, Vec::new())
            }
            Ok(matches) => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = matches
                    .first()
                    .map(|(name, score)| format!("{name}({score:.3})"));
                let matched_preview: Vec<String> = matches
                    .iter()
                    .map(|(n, s)| format!("{}({:.3})", n, s))
                    .collect();
                let names: Vec<String> = matches.into_iter().map(|(name, _)| name).collect();
                let router_match_count = names.len();
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_SEMANTIC_HIT,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, router_match_count),
                    turn_seq,
                    tools_needed = true,
                    router_match_count,
                    matched = ?matched_preview,
                    "semantic tool match; tool mode"
                );
                (true, names)
            }
            Err(e) => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = None;
                tracing::warn!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_MATCH_ERROR,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(true, 0),
                    turn_seq,
                    tools_needed = true,
                    router_match_count = 0usize,
                    fcp_error = %e,
                    "pre-LLM match_tools failed; roster-only tool mode"
                );
                (true, Vec::new())
            }
        }
    }
}
