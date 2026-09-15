//! Ordered routing policy rules (Phase 1 logic, Phase 2 shape).
//!
//! Rules run in a fixed order and emit a [`RoutingDecision`]. Embeddings stay
//! the primary signal; rules only veto, widen, or pair.

use super::clusters::{
    cluster_members, domains_share_affinity, tool_domain, union_clusters_for_tools,
};
use super::decision::{RoutingDecision, UnsureFallback};
use super::dialog::try_dialog_pairing;
use super::signals::RoutingSignals;

/// Lexical / forced hits use this floor so demotion never drops URL/search/news injects.
const FORCED_HIT_FLOOR: f32 = 0.99;

/// Knobs for demotion / margin / unsure fallback (from [`crate::config::AppConfig`]).
#[derive(Debug, Clone, Copy)]
pub struct RoutingPolicyKnobs {
    pub single_hit_floor: f32,
    pub match_margin: f32,
    pub unsure_fallback: UnsureFallback,
}

impl Default for RoutingPolicyKnobs {
    fn default() -> Self {
        Self {
            single_hit_floor: 0.58,
            match_margin: 0.05,
            unsure_fallback: UnsureFallback::FullRoster,
        }
    }
}

/// Apply ordered policy rules to signals → [`RoutingDecision`].
#[must_use]
pub fn decide(
    signals: &RoutingSignals,
    registered: &[String],
    knobs: RoutingPolicyKnobs,
) -> RoutingDecision {
    // Rule 1 — dialog pairing (agenda → mail → calendar → gated doc).
    if let Some(decision) = try_dialog_pairing(signals, registered) {
        return decision;
    }

    let (forced, embed): (Vec<_>, Vec<_>) = signals
        .embed_hits
        .iter()
        .cloned()
        .partition(|(_, score)| *score >= FORCED_HIT_FLOOR);

    // Rule 2 — lone weak embed hit demotion (+ unsure fallback).
    let (embed, demoted_lone) = demote_lone_weak_embed(embed, knobs.single_hit_floor);
    if let Some((demoted_name, _)) = demoted_lone {
        if embed.is_empty() && forced.is_empty() {
            return rule_unsure_after_demotion(&demoted_name, registered, knobs.unsure_fallback);
        }
    }

    // Rule 3 — near-tie across related domains → affinity cluster union; else ranked subset.
    let (embed_offer, margin_rule) =
        widen_near_tie_to_clusters(&embed, registered, knobs.match_margin);

    let mut offered = embed_offer;
    for (name, _) in &forced {
        if !offered.contains(name) {
            offered.push(name.clone());
        }
    }

    // Always re-rank by original cosine (no URL hard-pin).
    offered = rerank_offered_by_cosine(&offered, &signals.embed_hits);

    if offered.is_empty() {
        return RoutingDecision::full_roster("EMPTY_AFTER_POLICY");
    }

    let rule_id = if let Some(id) = margin_rule {
        id
    } else if forced.is_empty() && embed.len() == 1 {
        "SINGLE_STRONG_HIT"
    } else if !forced.is_empty() && embed.is_empty() {
        "LEXICAL_FORCED_ONLY"
    } else if !forced.is_empty() {
        "MIXED_EMBED_AND_LEXICAL"
    } else {
        "RANKED_SUBSET"
    };

    if rule_id == "AFFINITY_MARGIN_UNION" {
        let domains = domains_as_static(&offered);
        return RoutingDecision::domain_cluster(rule_id, domains, offered);
    }

    RoutingDecision::subset(rule_id, offered)
}

/// Compatibility wrapper used by older call sites / tests.
#[must_use]
pub fn apply_routing_policy(
    user_text: &str,
    hits: &[(String, f32)],
    recent_successful_tools: &[String],
    registered: &[String],
    knobs: RoutingPolicyKnobs,
) -> RoutingDecision {
    let signals = RoutingSignals::from_turn(user_text, hits.to_vec(), recent_successful_tools);
    decide(&signals, registered, knobs)
}

fn domains_as_static(tools: &[String]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for name in tools {
        if let Some(d) = tool_domain(name) {
            if let Some(s) = intern_known_domain(d) {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

fn intern_known_domain(d: &str) -> Option<&'static str> {
    Some(match d {
        "agenda" => "agenda",
        "clock" => "clock",
        "calendar" => "calendar",
        "web" => "web",
        "news" => "news",
        "wiki" => "wiki",
        "doc" => "doc",
        "vault" => "vault",
        "memory" => "memory",
        "media" => "media",
        "mail" => "mail",
        "weather" => "weather",
        "moltbook" => "moltbook",
        "db" => "db",
        "vision" => "vision",
        "system" => "system",
        "skills" => "skills",
        _ => return None,
    })
}

fn rule_unsure_after_demotion(
    demoted_name: &str,
    registered: &[String],
    fallback: UnsureFallback,
) -> RoutingDecision {
    match fallback {
        UnsureFallback::FullRoster => {
            tracing::info!(
                demoted = %demoted_name,
                fallback = "full_roster",
                event = "routing.policy.unsure_fallback",
                "Lone weak hit demoted; falling through to full roster"
            );
            RoutingDecision::full_roster("UNSURE_FULL_ROSTER")
        }
        UnsureFallback::DomainCluster => {
            if let Some(domain) = tool_domain(demoted_name).and_then(intern_known_domain) {
                let tools = cluster_members(domain, registered);
                if !tools.is_empty() {
                    tracing::info!(
                        demoted = %demoted_name,
                        domain,
                        offered_count = tools.len(),
                        fallback = "domain_cluster",
                        event = "routing.policy.unsure_fallback",
                        "Lone weak hit demoted; offering demoted tool's domain cluster"
                    );
                    return RoutingDecision::domain_cluster(
                        "UNSURE_DOMAIN_CLUSTER",
                        vec![domain],
                        tools,
                    );
                }
            }
            RoutingDecision::full_roster("UNSURE_FULL_ROSTER")
        }
    }
}

fn demote_lone_weak_embed(
    embed: Vec<(String, f32)>,
    single_hit_floor: f32,
) -> (Vec<(String, f32)>, Option<(String, f32)>) {
    if embed.len() == 1 && embed[0].1 < single_hit_floor {
        tracing::info!(
            tool = %embed[0].0,
            score = embed[0].1,
            floor = single_hit_floor,
            event = "routing.policy.single_hit_demoted",
            "Lone weak semantic hit demoted (avoid GBNF lock-in)"
        );
        let demoted = (embed[0].0.clone(), embed[0].1);
        return (Vec::new(), Some(demoted));
    }
    (embed, None)
}

/// Returns (offer names, optional rule id when affinity union fired).
fn widen_near_tie_to_clusters(
    embed: &[(String, f32)],
    registered: &[String],
    match_margin: f32,
) -> (Vec<String>, Option<&'static str>) {
    if embed.is_empty() {
        return (Vec::new(), None);
    }
    if embed.len() == 1 {
        return (vec![embed[0].0.clone()], None);
    }

    let top = embed[0].1;
    let within: Vec<(String, f32)> = embed
        .iter()
        .filter(|(_, s)| top - *s <= match_margin)
        .cloned()
        .collect();

    if within.len() < 2 {
        return (embed.iter().map(|(n, _)| n.clone()).collect(), None);
    }

    let Some(top_domain) = tool_domain(&within[0].0) else {
        return (embed.iter().map(|(n, _)| n.clone()).collect(), None);
    };

    let related_seeds: Vec<(String, f32)> = within
        .iter()
        .filter(|(n, _)| {
            tool_domain(n)
                .map(|d| domains_share_affinity(top_domain, d))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    let mut related_domains: Vec<&str> = related_seeds
        .iter()
        .filter_map(|(n, _)| tool_domain(n))
        .collect();
    related_domains.sort_unstable();
    related_domains.dedup();

    let mut all_within_domains: Vec<&str> =
        within.iter().filter_map(|(n, _)| tool_domain(n)).collect();
    all_within_domains.sort_unstable();
    all_within_domains.dedup();

    if related_domains.len() >= 2 {
        let seed: Vec<String> = related_seeds.iter().map(|(n, _)| n.clone()).collect();
        let union = union_clusters_for_tools(&seed, registered);
        tracing::info!(
            top_score = top,
            margin = match_margin,
            top_domain,
            related_domains = ?related_domains,
            skipped_unrelated = ?(all_within_domains
                .iter()
                .filter(|d| !related_domains.contains(d))
                .collect::<Vec<_>>()),
            offered_count = union.len(),
            event = "routing.policy.margin_multi_domain_union",
            "Near-tie across related domains; offering affinity cluster union"
        );
        return (union, Some("AFFINITY_MARGIN_UNION"));
    }

    if all_within_domains.len() >= 2 {
        tracing::info!(
            top_score = top,
            margin = match_margin,
            top_domain,
            within_domains = ?all_within_domains,
            event = "routing.policy.margin_unrelated_kept_ranked",
            "Near-tie spans unrelated domains; keeping cosine-ranked hits (no cluster dump)"
        );
    }

    (
        embed.iter().map(|(n, _)| n.clone()).collect(),
        Some("RANKED_SUBSET"),
    )
}

/// Order offer names by original router cosine. Unscored cluster siblings inherit
/// the best score among seed hits in the same domain (then name for stability).
#[must_use]
pub fn rerank_offered_by_cosine(offered: &[String], hits: &[(String, f32)]) -> Vec<String> {
    let mut domain_best: Vec<(&str, f32)> = Vec::new();
    for (name, score) in hits {
        if let Some(domain) = tool_domain(name) {
            if let Some((_, best)) = domain_best.iter_mut().find(|(d, _)| *d == domain) {
                if *score > *best {
                    *best = *score;
                }
            } else {
                domain_best.push((domain, *score));
            }
        }
    }

    let score_for = |name: &str| -> f32 {
        if let Some((_, s)) = hits.iter().find(|(n, _)| n == name) {
            return *s;
        }
        if let Some(domain) = tool_domain(name) {
            if let Some((_, best)) = domain_best.iter().find(|(d, _)| *d == domain) {
                return (*best) - 1.0e-4;
            }
        }
        0.0
    };

    let mut ranked = offered.to_vec();
    ranked.sort_by(|a, b| {
        let sa = score_for(a);
        let sb = score_for(b);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    ranked
}

/// True when the user message looks like it contains a URL / host.
#[must_use]
pub fn user_text_has_url(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    lower.contains("http://") || lower.contains("https://") || lower.contains("www.")
}

/// Prefer calling `web:fetch` this turn unless the user clearly wants opinion-only chatter.
///
/// Opinion-only (URL present, no fetch verbs): "do you know", "have you heard",
/// "what do you think", prior-knowledge phrasing — Idle with empty `tool_calls` is allowed
/// and we skip injecting [`URL_SOFT_COMPEL_HINT`]. Lexical URL inject may still offer fetch.
#[must_use]
pub fn should_soft_compel_web_fetch(user_text: &str) -> bool {
    if !user_text_has_url(user_text) {
        return false;
    }
    let lower = user_text.to_ascii_lowercase();
    if has_explicit_fetch_verb(&lower) {
        return true;
    }
    if is_opinion_only_url_chat(&lower) {
        return false;
    }
    true
}

fn has_explicit_fetch_verb(lower: &str) -> bool {
    [
        "fetch",
        "read ",
        "read it",
        "open ",
        "open it",
        "visit ",
        "summarize",
        "look at",
        "check out",
        "pull up",
    ]
    .iter()
    .any(|v| lower.contains(v))
}

/// Plan Phase-1/3: opinion without an explicit read/fetch ask.
fn is_opinion_only_url_chat(lower: &str) -> bool {
    const CUES: &[&str] = &[
        "do you know",
        "have you heard",
        "what do you think",
        "your opinion",
        "from your knowledge",
        "without reading",
        "without fetching",
        "without opening",
        "just tell me what you think",
    ];
    CUES.iter().any(|c| lower.contains(c))
}

/// System-prompt block when a URL is present and web:fetch is in the offer.
pub const URL_SOFT_COMPEL_HINT: &str = "\
[URL_TOOL_HINT] The latest user message includes a URL. Prefer calling `web:fetch` this turn \
(with `web:find` afterward if you need page content) instead of only discussing the link. \
Put narration in `message_to_user` after tools return. Opinion-only replies with empty \
`tool_calls` are allowed only if the user clearly asked for your prior knowledge without reading.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::routing::decision::{RoutingOffer, UnsureFallback};

    fn reg() -> Vec<String> {
        vec![
            "agenda:list".into(),
            "agenda:remove".into(),
            "agenda:complete".into(),
            "agenda:remind_at".into(),
            "agenda:push".into(),
            "clock:now".into(),
            "clock:alarm".into(),
            "clock:timer".into(),
            "doc:ingest".into(),
            "doc:query".into(),
            "doc:delete".into(),
            "doc:list".into(),
            "doc:read".into(),
            "web:fetch".into(),
            "web:find".into(),
            "web:search".into(),
            "memory:query".into(),
            "memory:commit".into(),
            "memory:stage".into(),
            "memory:staged_list".into(),
            "memory:commit_all".into(),
            "moltbook:search".into(),
            "moltbook:home".into(),
            "db:find_connections".into(),
        ]
    }

    #[test]
    fn lone_weak_doc_ingest_demoted_to_full_roster() {
        let hits = vec![("doc:ingest".into(), 0.505)];
        let out = apply_routing_policy(
            "remove it",
            &hits,
            &[],
            &reg(),
            RoutingPolicyKnobs::default(),
        );
        assert!(matches!(out.offer, RoutingOffer::FullRoster), "{out:?}");
        assert!(out.matched_tool_names().is_empty());
    }

    #[test]
    fn lone_weak_unsure_domain_cluster_fallback() {
        let hits = vec![("doc:ingest".into(), 0.505)];
        let knobs = RoutingPolicyKnobs {
            unsure_fallback: UnsureFallback::DomainCluster,
            ..RoutingPolicyKnobs::default()
        };
        let out = apply_routing_policy("remove it", &hits, &[], &reg(), knobs);
        assert_eq!(out.rule_id, "UNSURE_DOMAIN_CLUSTER");
        match &out.offer {
            RoutingOffer::DomainCluster { domains, tools } => {
                assert_eq!(domains.as_slice(), ["doc"]);
                assert!(tools.iter().all(|n| n.starts_with("doc:")));
            }
            other => panic!("expected domain cluster, got {other:?}"),
        }
    }

    #[test]
    fn lone_weak_after_agenda_pairs_to_agenda_cluster() {
        let hits = vec![("doc:ingest".into(), 0.505)];
        let recent = vec!["agenda:list".into()];
        let out = apply_routing_policy(
            "yes remove it from the agenda",
            &hits,
            &recent,
            &reg(),
            RoutingPolicyKnobs::default(),
        );
        assert_eq!(out.rule_id, "AGENDA_DIALOG_PAIRING");
        let names = out.matched_tool_names();
        assert!(names.iter().any(|n| n == "agenda:remove"));
        assert!(!names.iter().any(|n| n.starts_with("doc:")));
    }

    #[test]
    fn strong_single_hit_kept() {
        let hits = vec![("vault:read".into(), 0.72)];
        let out = apply_routing_policy(
            "read notes/today.md",
            &hits,
            &[],
            &reg(),
            RoutingPolicyKnobs::default(),
        );
        assert_eq!(out.matched_tool_names(), vec!["vault:read".to_string()]);
        assert_eq!(out.rule_id, "SINGLE_STRONG_HIT");
    }

    #[test]
    fn clock_agenda_near_tie_unions_clusters_reranked_by_cosine() {
        let hits = vec![
            ("clock:alarm".into(), 0.61),
            ("agenda:remind_at".into(), 0.59),
        ];
        let out = apply_routing_policy(
            "remind me tomorrow at 10",
            &hits,
            &[],
            &reg(),
            RoutingPolicyKnobs::default(),
        );
        assert_eq!(out.rule_id, "AFFINITY_MARGIN_UNION");
        let names = out.matched_tool_names();
        assert!(names.contains(&"clock:timer".to_string()));
        assert!(names.contains(&"agenda:list".to_string()));
        assert!(!names.contains(&"doc:ingest".to_string()));
        // Seed hit leads; unscored clock:* siblings inherit ~top and may rank before agenda.
        assert_eq!(names[0], "clock:alarm");
        let agenda_pos = names
            .iter()
            .position(|n| n == "agenda:remind_at")
            .expect("agenda:remind_at in affinity union");
        assert!(
            agenda_pos > 0,
            "agenda seed should follow clock:alarm, got {names:?}"
        );
        // All clock:* from the union should still beat the agenda seed score (0.59).
        for (i, n) in names.iter().enumerate() {
            if n.starts_with("clock:") {
                assert!(
                    i < agenda_pos,
                    "clock sibling {n} should rank before agenda:remind_at, got {names:?}"
                );
            }
        }
    }

    #[test]
    fn unrelated_near_tie_mush_keeps_ranked_no_db_first() {
        let hits = vec![
            ("moltbook:search".into(), 0.586),
            ("web:search".into(), 0.569),
            ("memory:query".into(), 0.562),
            ("doc:query".into(), 0.548),
            ("memory:commit".into(), 0.543),
            ("web:fetch".into(), 0.542),
            ("db:find_connections".into(), 0.540),
            ("web:find".into(), 0.537),
        ];
        let out = apply_routing_policy(
            "do you know this repository online? https://github.com/vllm-project/semantic-router",
            &hits,
            &[],
            &reg(),
            RoutingPolicyKnobs::default(),
        );
        let names = out.matched_tool_names();
        assert_eq!(names[0], "moltbook:search");
        assert!(!names.iter().any(|n| n == "doc:ingest"));
        assert!(names.iter().any(|n| n == "web:fetch"));
        let db_pos = names
            .iter()
            .position(|n| n == "db:find_connections")
            .unwrap();
        let fetch_pos = names.iter().position(|n| n == "web:fetch").unwrap();
        assert!(
            fetch_pos < db_pos,
            "cosine order: fetch before db, got {names:?}"
        );
    }

    #[test]
    fn lexical_forced_fetch_survives_demotion() {
        let hits = vec![
            ("doc:ingest".into(), 0.505),
            ("web:fetch".into(), 1.0),
            ("web:find".into(), 0.99),
        ];
        let out = apply_routing_policy(
            "https://example.com/x",
            &hits,
            &[],
            &reg(),
            RoutingPolicyKnobs::default(),
        );
        let names = out.matched_tool_names();
        assert!(names.iter().any(|n| n == "web:fetch"));
        assert!(!names.iter().any(|n| n == "doc:ingest"));
        assert_eq!(names[0], "web:fetch");
    }

    #[test]
    fn soft_compel_true_for_fetch_ask_with_url() {
        assert!(should_soft_compel_web_fetch(
            "please open https://github.com/vllm-project/semantic-router and summarize"
        ));
        assert!(should_soft_compel_web_fetch(
            "check out https://eris-system.dev"
        ));
        // Bare URL / non-opinion phrasing still soft-compels.
        assert!(should_soft_compel_web_fetch(
            "https://github.com/vllm-project/semantic-router"
        ));
    }

    #[test]
    fn soft_compel_false_for_opinion_only_with_url() {
        assert!(!should_soft_compel_web_fetch(
            "do you know this repo? https://github.com/vllm-project/semantic-router"
        ));
        assert!(!should_soft_compel_web_fetch(
            "have you heard of https://example.com — what do you think?"
        ));
        // Fetch verb wins over opinion cue.
        assert!(should_soft_compel_web_fetch(
            "do you know this site? please fetch https://example.com anyway"
        ));
    }
}
