//! Offline routing-offer fixtures (Phase 3 lean).
//!
//! Each case is: user text + synthetic router hits + recent successes
//! → expected `rule_id` and offer constraints.
//!
//! **No live llama / embeddings** — hits are fixed so fixtures stay stable.
//! Live cosine quality still needs a real chat pass after hint edits.
//!
//! Run via `cargo test routing_offer_fixtures` (operator: avoid if the
//! heisenbug is active on this host).

use crate::orchestrator::routing::{
    RoutingOffer, RoutingPolicyKnobs, UnsureFallback, apply_routing_policy,
};

/// One golden case for pre-LLM offer policy.
#[derive(Debug, Clone)]
pub struct RoutingOfferFixture {
    pub id: &'static str,
    pub user_text: &'static str,
    pub hits: &'static [(&'static str, f32)],
    pub recent_successful: &'static [&'static str],
    pub expected_rule_id: &'static str,
    /// First offered tool name (when non-empty offer).
    pub expect_first: Option<&'static str>,
    /// Must appear somewhere in the offer.
    pub must_include: &'static [&'static str],
    /// Must not appear in the offer.
    pub must_exclude: &'static [&'static str],
    /// When true, offer names must be empty (full roster / conversational empty).
    pub expect_empty_names: bool,
    pub unsure_fallback: UnsureFallback,
}

fn fixture_registry() -> Vec<String> {
    [
        "agenda:list",
        "agenda:remove",
        "agenda:complete",
        "agenda:remind_at",
        "agenda:push",
        "agenda:remind_self",
        "clock:now",
        "clock:alarm",
        "clock:timer",
        "calendar:list",
        "calendar:get",
        "calendar:delete",
        "calendar:update",
        "calendar:create",
        "doc:ingest",
        "doc:query",
        "doc:delete",
        "doc:list",
        "doc:read",
        "web:fetch",
        "web:find",
        "web:search",
        "mail:check",
        "mail:read",
        "mail:delete",
        "mail:move",
        "mail:write",
        "mail:digest",
        "memory:query",
        "memory:commit",
        "memory:stage",
        "memory:staged_list",
        "memory:commit_all",
        "moltbook:search",
        "moltbook:home",
        "db:find_connections",
        "vault:read",
        "news:today",
        "wiki:summary",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// Canonical fixture table (extend here when adding routing rules).
pub fn all_routing_offer_fixtures() -> Vec<RoutingOfferFixture> {
    vec![
        RoutingOfferFixture {
            id: "demote_lone_weak_doc_ingest",
            user_text: "please remove that thing from earlier",
            hits: &[("doc:ingest", 0.505)],
            recent_successful: &[],
            expected_rule_id: "UNSURE_FULL_ROSTER",
            expect_first: None,
            must_include: &[],
            must_exclude: &[],
            expect_empty_names: true,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "unsure_domain_cluster_after_demotion",
            user_text: "please remove that thing from earlier",
            hits: &[("doc:ingest", 0.505)],
            recent_successful: &[],
            expected_rule_id: "UNSURE_DOMAIN_CLUSTER",
            expect_first: None, // alphabetical cluster; assert all doc:* below
            must_include: &["doc:ingest", "doc:list", "doc:delete"],
            must_exclude: &["agenda:remove"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::DomainCluster,
        },
        RoutingOfferFixture {
            id: "agenda_dialog_after_list",
            user_text: "please remove that oat milk reminder from my agenda now",
            hits: &[("doc:ingest", 0.505), ("doc:delete", 0.548)],
            recent_successful: &["agenda:list"],
            expected_rule_id: "AGENDA_DIALOG_PAIRING",
            expect_first: Some("agenda:remove"),
            must_include: &["agenda:complete", "agenda:list"],
            must_exclude: &["doc:ingest", "doc:delete"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "mail_delete_after_check",
            user_text: "please delete that email from my inbox",
            hits: &[("doc:delete", 0.52), ("agenda:remove", 0.51)],
            recent_successful: &["mail:check"],
            expected_rule_id: "MAIL_DIALOG_PAIRING",
            expect_first: Some("mail:delete"),
            must_include: &["mail:check"],
            must_exclude: &["doc:delete"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "calendar_cancel_after_list",
            user_text: "please cancel that meeting on my calendar",
            hits: &[("agenda:remove", 0.55), ("clock:alarm", 0.54)],
            recent_successful: &["calendar:list"],
            expected_rule_id: "CALENDAR_DIALOG_PAIRING",
            expect_first: Some("calendar:delete"),
            must_include: &["calendar:list"],
            must_exclude: &["agenda:remove", "clock:alarm"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "doc_anchored_delete_after_list",
            user_text: "please delete that document from the ingested store",
            hits: &[("agenda:remove", 0.52)],
            recent_successful: &["doc:list"],
            expected_rule_id: "DOC_DIALOG_PAIRING",
            expect_first: Some("doc:delete"),
            must_include: &["doc:list"],
            must_exclude: &["agenda:remove"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "bare_remove_after_doc_list_no_doc_pair",
            user_text: "please remove it",
            hits: &[("doc:delete", 0.51)],
            recent_successful: &["doc:list"],
            // No dialog pairing → lone weak may demote to full roster
            expected_rule_id: "UNSURE_FULL_ROSTER",
            expect_first: None,
            must_include: &[],
            must_exclude: &[],
            expect_empty_names: true,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "clock_agenda_affinity_union",
            user_text: "please remind me tomorrow morning at ten about the dentist appointment",
            hits: &[("clock:alarm", 0.61), ("agenda:remind_at", 0.59)],
            recent_successful: &[],
            expected_rule_id: "AFFINITY_MARGIN_UNION",
            expect_first: Some("clock:alarm"),
            must_include: &["agenda:remind_at", "clock:timer", "agenda:list"],
            must_exclude: &["doc:ingest", "db:find_connections"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "url_mush_keeps_ranked_no_db_first",
            user_text: "do you know this repository online? https://github.com/vllm-project/semantic-router",
            hits: &[
                ("moltbook:search", 0.586),
                ("web:search", 0.569),
                ("memory:query", 0.562),
                ("doc:query", 0.548),
                ("memory:commit", 0.543),
                ("web:fetch", 0.542),
                ("db:find_connections", 0.540),
                ("web:find", 0.537),
            ],
            recent_successful: &[],
            expected_rule_id: "RANKED_SUBSET",
            expect_first: Some("moltbook:search"),
            must_include: &["web:fetch", "db:find_connections"],
            must_exclude: &["doc:ingest"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "lexical_fetch_leads_cosine_rank",
            user_text: "please open this page and summarize: https://eris-system.dev",
            hits: &[
                ("doc:ingest", 0.505),
                ("web:fetch", 1.0),
                ("web:find", 0.99),
            ],
            recent_successful: &[],
            // Lone weak embed demoted; forced lexical hits survive.
            expected_rule_id: "LEXICAL_FORCED_ONLY",
            expect_first: Some("web:fetch"),
            must_include: &["web:find"],
            must_exclude: &["doc:ingest"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "mixed_embed_and_lexical_fetch",
            user_text: "please summarize https://eris-system.dev and relate it to my notes",
            hits: &[
                ("memory:query", 0.62),
                ("web:fetch", 1.0),
                ("web:find", 0.99),
            ],
            recent_successful: &[],
            expected_rule_id: "MIXED_EMBED_AND_LEXICAL",
            expect_first: Some("web:fetch"),
            must_include: &["memory:query", "web:find"],
            must_exclude: &[],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "strong_single_vault_read",
            user_text: "please read the vault note at notes/today.md if it exists",
            hits: &[("vault:read", 0.72)],
            recent_successful: &[],
            expected_rule_id: "SINGLE_STRONG_HIT",
            expect_first: Some("vault:read"),
            must_include: &["vault:read"],
            must_exclude: &[],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "agenda_beats_mail_when_both_recent",
            user_text: "please remove that from my agenda",
            hits: &[("mail:delete", 0.60)],
            recent_successful: &["mail:check", "agenda:list"],
            expected_rule_id: "AGENDA_DIALOG_PAIRING",
            expect_first: Some("agenda:remove"),
            must_include: &["agenda:list"],
            must_exclude: &["mail:delete"],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "weather_strong_ranked",
            user_text: "what is the current weather forecast for Berlin Germany right now",
            hits: &[
                ("weather:current", 0.704),
                ("weather:forecast", 0.697),
                ("db:find_connections", 0.652),
            ],
            recent_successful: &[],
            expected_rule_id: "RANKED_SUBSET",
            expect_first: Some("weather:current"),
            must_include: &["weather:forecast"],
            must_exclude: &[],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "mail_check_ranked",
            user_text: "please check whether I have any unread email in my inbox",
            hits: &[
                ("mail:check", 0.705),
                ("mail:read", 0.617),
                ("mail:digest", 0.568),
            ],
            recent_successful: &[],
            expected_rule_id: "RANKED_SUBSET",
            expect_first: Some("mail:check"),
            must_include: &["mail:read"],
            must_exclude: &[],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
        RoutingOfferFixture {
            id: "doc_list_strong_ranked",
            user_text: "please list all of my ingested documents in the document store",
            hits: &[
                ("doc:list", 0.789),
                ("doc:ingest", 0.739),
                ("doc:delete", 0.685),
            ],
            recent_successful: &[],
            expected_rule_id: "RANKED_SUBSET",
            expect_first: Some("doc:list"),
            must_include: &["doc:ingest"],
            must_exclude: &[],
            expect_empty_names: false,
            unsure_fallback: UnsureFallback::FullRoster,
        },
    ]
}

/// Evaluate one fixture; returns `Ok(())` or an error string.
pub fn eval_routing_offer_fixture(fx: &RoutingOfferFixture) -> Result<(), String> {
    let hits: Vec<(String, f32)> = fx
        .hits
        .iter()
        .map(|(n, s)| ((*n).to_string(), *s))
        .collect();
    let recent: Vec<String> = fx
        .recent_successful
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let mut reg = fixture_registry();
    // Ensure weather tools exist for weather fixture.
    for w in ["weather:current", "weather:forecast"] {
        if !reg.iter().any(|n| n == w) {
            reg.push(w.to_string());
        }
    }

    let knobs = RoutingPolicyKnobs {
        single_hit_floor: 0.58,
        match_margin: 0.05,
        unsure_fallback: fx.unsure_fallback,
    };
    let decision = apply_routing_policy(fx.user_text, &hits, &recent, &reg, knobs);
    let names = decision.matched_tool_names();

    if decision.rule_id != fx.expected_rule_id {
        return Err(format!(
            "[{}] rule_id: got {}, want {}",
            fx.id, decision.rule_id, fx.expected_rule_id
        ));
    }
    if fx.expect_empty_names {
        if !names.is_empty() {
            return Err(format!(
                "[{}] expected empty names (full roster), got {names:?}",
                fx.id
            ));
        }
        return Ok(());
    }
    if let Some(first) = fx.expect_first {
        match names.first() {
            Some(n) if n == first => {}
            other => {
                return Err(format!(
                    "[{}] expect_first={first}, got {other:?} (full={names:?})",
                    fx.id
                ));
            }
        }
    }
    for must in fx.must_include {
        if !names.iter().any(|n| n == must) {
            return Err(format!(
                "[{}] missing must_include {must} in {names:?}",
                fx.id
            ));
        }
    }
    for bad in fx.must_exclude {
        if names.iter().any(|n| n == bad) {
            return Err(format!(
                "[{}] must_exclude {bad} present in {names:?}",
                fx.id
            ));
        }
    }
    // Ranked URL mush: fetch before db when both present.
    if fx.id == "url_mush_keeps_ranked_no_db_first" {
        let fetch = names.iter().position(|n| n == "web:fetch");
        let db = names.iter().position(|n| n == "db:find_connections");
        match (fetch, db) {
            (Some(f), Some(d)) if f < d => {}
            other => {
                return Err(format!(
                    "[{}] web:fetch should rank before db:find_connections, got {other:?} in {names:?}",
                    fx.id
                ));
            }
        }
    }
    // Domain-cluster demotion: first may be any doc:* after cosine re-rank of cluster.
    if fx.id == "unsure_domain_cluster_after_demotion" {
        if !matches!(decision.offer, RoutingOffer::DomainCluster { .. }) {
            return Err(format!("[{}] expected DomainCluster offer", fx.id));
        }
        if !names.iter().all(|n| n.starts_with("doc:")) {
            return Err(format!(
                "[{}] expected only doc:* tools, got {names:?}",
                fx.id
            ));
        }
    }
    Ok(())
}

/// Run the full table; returns `(passed, failures)`.
pub fn run_all_routing_offer_fixtures() -> (usize, Vec<String>) {
    let mut passed = 0usize;
    let mut failures = Vec::new();
    for fx in all_routing_offer_fixtures() {
        match eval_routing_offer_fixture(&fx) {
            Ok(()) => passed += 1,
            Err(e) => failures.push(e),
        }
    }
    (passed, failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_routing_offer_fixtures_pass() {
        let (passed, failures) = run_all_routing_offer_fixtures();
        assert!(
            failures.is_empty(),
            "routing offer fixtures: {passed} passed, failures:\n{}",
            failures.join("\n")
        );
        assert!(passed >= 12, "expected at least 12 fixtures, got {passed}");
    }
}
