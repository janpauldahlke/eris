use crate::engine::LlmEngine;
use crate::orchestrator::buffer_continuation::{
    buffer_followup_routing_appendix, stack_has_buffer_routing_context,
};
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

        if ToolRouter::short_input_guard_conversational_only(user_input, &self.chat_stack) {
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

        let embed_input = self.compose_tool_routing_embed_input(user_input);
        let router_started = Instant::now();
        match router
            .match_tools(&embed_input, user_input)
            .await
        {
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

    /// Enrich the router embedding when the user is continuing a staged large read (model-driven follow-ups).
    fn compose_tool_routing_embed_input(&self, user_input: &str) -> String {
        let mut out = user_input.to_string();
        let tail_wanted = stack_has_buffer_routing_context(&self.chat_stack)
            && (ToolRouter::has_buffer_continuation_lexical_intent(user_input)
                || ToolRouter::is_short_buffer_followup_ack(user_input));
        if !tail_wanted {
            return out;
        }
        if let Some(snippet) = buffer_followup_routing_appendix(&self.chat_stack) {
            out.push_str(
                "\n\n[FCP routing context: large content is in an ephemeral buffer. Prefer the `[FCP BUFFER SESSION]` block below for `buffer_id`, `last_page`, and `next_page`. Otherwise use `buffer_id` from vault:read / web:fetch receipts (short handle such as buf_1). Call `ephemeral:buffer_page` with the same `buffer_id` and `next_page` (or increment `page`); use `ephemeral:buffer_query` for keyword search. Copy the token exactly—do not invent or paraphrase it.]\n",
            );
            out.push_str(&snippet);
        }
        out
    }
}
