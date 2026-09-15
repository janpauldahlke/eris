//! Slim-offer overlays shared by prompt assembly and GBNF subset selection.
//!
//! Single source of truth for: offer cap, **domain verb completion**, Moltbook latch,
//! `web:find` pairing, `doc:read` → `vault:write`, and `vision:see` → `media:catalog`.
//!
//! ## Domain verb completion
//!
//! The ranked router list + blind `.take(cap)` often keeps one verb of a domain
//! (`vault:search`) while dropping siblings (`vault:write`). When a domain appears
//! among the highest-ranked tools that earn a seat under the cap, we complete that
//! domain's full state-allowed verb set and keep those verbs together (a resort),
//! even if the completed set exceeds `tool_map_offer_cap`. Cap still decides which
//! *domains* get a seat; completion decides that a seated domain is not verb-amputated.
//! See `docs/TODO/TOOL_OFFER_CAP_DROPS_WRITES.md`.

use super::clusters::{cluster_members, tool_domain};
use crate::orchestrator::state::AgentState;
use crate::tools::Gatekeeper;

/// Build the final offered-tool list for slim phrase map + subset grammar.
#[must_use]
pub fn apply_offer_overlays(
    pre_llm_matched_tools: &[String],
    tool_map_offer_cap: usize,
    moltbook_overlay_latched: bool,
    gatekeeper: &Gatekeeper,
    state: &AgentState,
) -> Vec<String> {
    let registered = gatekeeper.registered_tool_names();

    // Seeds that earn a domain seat: highest-ranked prefix under the cap (or all if uncapped).
    let seed_limit = if tool_map_offer_cap == 0 {
        pre_llm_matched_tools.len()
    } else {
        tool_map_offer_cap.min(pre_llm_matched_tools.len())
    };
    let seeds = &pre_llm_matched_tools[..seed_limit];

    let mut offered = complete_seed_domain_verbs(seeds, &registered, state);

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

    // Remembering an image is always vision:see → media:catalog. media:catalog is a
    // persist tool that embeds just below generic read/query tools, so the offer cap
    // frequently truncates it out of the ranked subset (see docs/TODO/
    // TOOL_OFFER_CAP_DROPS_WRITES.md). Pair it with vision:see so the catalog step is
    // always reachable whenever vision is on the table.
    if offered.iter().any(|n| n == "vision:see")
        && !offered.iter().any(|n| n == "media:catalog")
        && Gatekeeper::state_allows_tool(state, "media:catalog")
    {
        offered.push("media:catalog".to_string());
    }

    offered
}

/// For each domain represented in `seeds` (rank order), emit that domain's full
/// state-allowed verb set: seed tools first (preserving relative rank), then any
/// remaining registered siblings (stable sort from [`cluster_members`]).
fn complete_seed_domain_verbs(
    seeds: &[String],
    registered: &[String],
    state: &AgentState,
) -> Vec<String> {
    if seeds.is_empty() {
        return Vec::new();
    }

    let mut domains_in_order: Vec<&str> = Vec::new();
    for name in seeds {
        if let Some(domain) = tool_domain(name) {
            if !domains_in_order.contains(&domain) {
                domains_in_order.push(domain);
            }
        }
    }

    let mut offered: Vec<String> = Vec::new();
    let mut included: std::collections::HashSet<String> = std::collections::HashSet::new();
    let before_seed_count = seeds.len();

    for domain in &domains_in_order {
        // Seed hits for this domain, in original rank order.
        for name in seeds {
            if tool_domain(name) == Some(*domain) && included.insert(name.clone()) {
                offered.push(name.clone());
            }
        }
        // Remaining verbs registered under this prefix.
        for sibling in cluster_members(domain, registered) {
            if !Gatekeeper::state_allows_tool(state, &sibling) {
                continue;
            }
            if included.insert(sibling.clone()) {
                offered.push(sibling);
            }
        }
    }

    // Tools without a domain prefix (should be rare) keep their seed seat.
    for name in seeds {
        if tool_domain(name).is_none() && included.insert(name.clone()) {
            offered.push(name.clone());
        }
    }

    if offered.len() > before_seed_count {
        tracing::info!(
            event = "routing.offer.domain_verb_complete",
            domains = ?domains_in_order,
            seed_count = before_seed_count,
            offered_count = offered.len(),
            "Completed domain verb sets for capped seed domains"
        );
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
    use std::sync::Arc;

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    struct NamedStub(&'static str);

    #[async_trait]
    impl Tool for NamedStub {
        fn name(&self) -> &'static str {
            self.0
        }

        fn description(&self) -> &'static str {
            "test"
        }

        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schema_for!(EmptyArgs)
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> crate::executive::error::Result<String> {
            Ok("{}".to_string())
        }
    }

    fn register_vault_memory(gk: &mut Gatekeeper) {
        for name in [
            "vault:search",
            "vault:read",
            "vault:write",
            "vault:list",
            "vault:taglist",
            "memory:query",
            "memory:stage",
            "memory:commit",
            "memory:commit_all",
            "memory:staged_list",
            "doc:ingest",
            "media:catalog",
            "web:fetch",
            "web:search",
            "web:find",
        ] {
            gk.register(Arc::new(NamedStub(name)));
        }
    }

    #[test]
    fn vault_seed_completes_all_vault_verbs_past_cap() {
        let mut gk = Gatekeeper::new();
        register_vault_memory(&mut gk);

        // Turn-15 shape: vault:search seats the vault domain; write must ride along
        // even when the ranked prefix is already at cap.
        let pre = vec![
            "memory:query".into(),
            "vault:search".into(),
            "memory:stage".into(),
            "memory:commit_all".into(),
            "doc:ingest".into(),
            "media:catalog".into(),
            "web:fetch".into(),
            "web:search".into(),
        ];
        let out = apply_offer_overlays(&pre, 8, false, &gk, &AgentState::Chat);

        assert!(
            out.contains(&"vault:write".to_string()),
            "vault:write must be offered when vault:search seats the vault domain; got {out:?}"
        );
        assert!(out.contains(&"vault:read".to_string()));
        assert!(out.contains(&"vault:list".to_string()));
        assert!(out.contains(&"vault:taglist".to_string()));
        assert!(out.contains(&"memory:staged_list".to_string()));
        // Cap no longer amputates seated domains; offer grows past 8.
        assert!(
            out.len() > 8,
            "domain completion may exceed cap; got len={}",
            out.len()
        );
    }

    #[test]
    fn domain_verbs_group_after_first_seed_of_domain() {
        let mut gk = Gatekeeper::new();
        register_vault_memory(&mut gk);

        let pre = vec!["vault:search".into(), "memory:query".into()];
        let out = apply_offer_overlays(&pre, 8, false, &gk, &AgentState::Chat);

        let search_i = out.iter().position(|n| n == "vault:search").expect("search");
        let write_i = out.iter().position(|n| n == "vault:write").expect("write");
        let query_i = out.iter().position(|n| n == "memory:query").expect("query");
        assert!(
            search_i < write_i && write_i < query_i,
            "vault verbs should complete before the next domain; got {out:?}"
        );
    }

    #[test]
    fn unrelated_domain_not_completed_when_absent_from_seeds() {
        let mut gk = Gatekeeper::new();
        register_vault_memory(&mut gk);

        let pre = vec!["web:fetch".into(), "web:search".into()];
        let out = apply_offer_overlays(&pre, 5, false, &gk, &AgentState::Chat);

        assert!(out.contains(&"web:find".to_string())); // pairing overlay
        assert!(!out.contains(&"vault:write".to_string()));
        assert!(!out.contains(&"memory:stage".to_string()));
    }

    #[test]
    fn uncapped_still_completes_seed_domains() {
        let mut gk = Gatekeeper::new();
        register_vault_memory(&mut gk);

        let pre = vec!["vault:search".into()];
        let out = apply_offer_overlays(&pre, 0, false, &gk, &AgentState::Chat);
        assert!(out.contains(&"vault:write".to_string()));
        assert_eq!(out.iter().filter(|n| n.starts_with("vault:")).count(), 5);
    }
}
