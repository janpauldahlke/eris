use crate::engine::LlmEngine;
use crate::orchestrator::routing::{
    RoutingDecision, RoutingPolicyKnobs, UnsureFallback, apply_routing_policy,
};
use crate::orchestrator::tool_router::ToolRouter;
use crate::presentation::SYSTEM_ALARM_PREFIX;
use crate::telemetry::routing_codes;
use std::time::Instant;

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    /// Conversational vs tool mode, plus a structured [`RoutingDecision`] for Tier 1.
    pub(super) async fn run_pre_llm_routing(&mut self) -> RoutingDecision {
        let user_input = self.last_user_content().to_string();
        let turn_seq = self.turn_seq;

        if user_input.starts_with(SYSTEM_ALARM_PREFIX) {
            let alarm_payload = user_input
                .strip_prefix(SYSTEM_ALARM_PREFIX)
                .unwrap_or(user_input.as_str());
            if alarm_payload.to_ascii_lowercase().contains("moltbook") {
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_ALARM_TOOL_ELIGIBLE,
                    outcome = routing_codes::OUTCOME_TOOL_FALLBACK,
                    turn_seq,
                    rule_id = "ALARM_MOLTBOOK",
                    offer_kind = "full_roster",
                    tools_needed = true,
                    router_match_count = 0usize,
                    "system alarm prefix with Moltbook label; semantic tool routing enabled"
                );
                // Fall through to normal router (tools needed).
            } else {
                self.last_router_ms = 0;
                self.last_top_tool_match = None;
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_CONV_ALARM,
                    outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                    turn_seq,
                    rule_id = "CONV_ALARM",
                    offer_kind = "conversational",
                    tools_needed = false,
                    router_match_count = 0usize,
                    "system alarm prefix; conversational mode"
                );
                return RoutingDecision::conversational("CONV_ALARM");
            }
        }

        if ToolRouter::short_input_guard_conversational_only(&user_input) {
            self.last_router_ms = 0;
            self.last_top_tool_match = None;
            tracing::info!(
                category = routing_codes::CATEGORY_ROUTING,
                issue = routing_codes::ISSUE_PRELLM_CONV_SHORT_INPUT,
                outcome = routing_codes::OUTCOME_CONVERSATIONAL,
                turn_seq,
                rule_id = "SHORT_INPUT",
                offer_kind = "conversational",
                tools_needed = false,
                router_match_count = 0usize,
                "short-input guard; conversational mode"
            );
            return RoutingDecision::conversational("SHORT_INPUT");
        }

        let router_started = Instant::now();
        let match_result = {
            let Some(router) = self.tool_router.as_ref() else {
                self.last_router_ms = 0;
                self.last_top_tool_match = None;
                tracing::warn!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_ROUTER_UNAVAILABLE,
                    outcome = routing_codes::OUTCOME_TOOL_FALLBACK,
                    turn_seq,
                    rule_id = "ROUTER_UNAVAILABLE",
                    offer_kind = "full_roster",
                    tools_needed = true,
                    router_match_count = 0usize,
                    "no tool router; roster-only tool mode"
                );
                return RoutingDecision::full_roster("ROUTER_UNAVAILABLE");
            };
            router.match_tools(&user_input).await
        };

        match match_result {
            Ok(matches) if matches.is_empty() => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = None;
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_SEMANTIC_EMPTY,
                    outcome = routing_codes::OUTCOME_TOOL_FALLBACK,
                    turn_seq,
                    rule_id = "SEMANTIC_EMPTY",
                    offer_kind = "full_roster",
                    tools_needed = true,
                    router_match_count = 0usize,
                    "no semantic tool match; tool fallback mode"
                );
                RoutingDecision::full_roster("SEMANTIC_EMPTY")
            }
            Ok(matches) => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                let raw_preview: Vec<String> = matches
                    .iter()
                    .map(|(n, s)| format!("{}({:.3})", n, s))
                    .collect();
                let registered = self.gatekeeper.registered_tool_names();
                let knobs = RoutingPolicyKnobs {
                    single_hit_floor: self.config.tool_single_hit_floor,
                    match_margin: self.config.tool_match_margin,
                    unsure_fallback: UnsureFallback::parse(&self.config.tool_unsure_fallback),
                };
                let recent = self.recent_successful_tools.clone();
                let decision =
                    apply_routing_policy(&user_input, &matches, &recent, &registered, knobs);
                let names = decision.matched_tool_names();
                self.last_top_tool_match = names
                    .first()
                    .cloned()
                    .or_else(|| matches.first().map(|(n, s)| format!("{n}({s:.3})")));
                let router_match_count = names.len();
                let issue = match decision.rule_id {
                    "SINGLE_STRONG_HIT"
                    | "RANKED_SUBSET"
                    | "LEXICAL_FORCED_ONLY"
                    | "MIXED_EMBED_AND_LEXICAL" => routing_codes::ISSUE_PRELLM_SEMANTIC_HIT,
                    _ => routing_codes::ISSUE_PRELLM_POLICY_REWRITE,
                };
                tracing::info!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue,
                    outcome = routing_codes::outcome_from_pre_llm_tuple(
                        decision.tools_needed(),
                        router_match_count
                    ),
                    turn_seq,
                    rule_id = decision.rule_id,
                    offer_kind = decision.offer.kind_label(),
                    tools_needed = decision.tools_needed(),
                    router_match_count,
                    raw_matched = ?raw_preview,
                    offered = ?names,
                    "pre-LLM routing decision"
                );
                decision
            }
            Err(e) => {
                self.last_router_ms = router_started.elapsed().as_millis() as u64;
                self.last_top_tool_match = None;
                tracing::warn!(
                    category = routing_codes::CATEGORY_ROUTING,
                    issue = routing_codes::ISSUE_PRELLM_MATCH_ERROR,
                    outcome = routing_codes::OUTCOME_TOOL_FALLBACK,
                    turn_seq,
                    rule_id = "MATCH_ERROR",
                    offer_kind = "full_roster",
                    tools_needed = true,
                    router_match_count = 0usize,
                    fcp_error = %e,
                    "pre-LLM match_tools failed; roster-only tool mode"
                );
                RoutingDecision::full_roster("MATCH_ERROR")
            }
        }
    }

    /// Record a successful tool for dialog-continuation routing (session-scoped, capped).
    pub(super) fn record_successful_tool(&mut self, tool_name: &str) {
        const CAP: usize = 12;
        self.recent_successful_tools.push(tool_name.to_string());
        if self.recent_successful_tools.len() > CAP {
            let drain = self.recent_successful_tools.len() - CAP;
            self.recent_successful_tools.drain(0..drain);
        }
    }
}
