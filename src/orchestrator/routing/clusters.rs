//! Domain clusters for Phase-1 tool-routing policy.
//!
//! When the semantic router is unsure (weak lone hit, near-tie across families),
//! we widen the offer to a prefix cluster instead of locking GBNF onto one tool.

/// Domain key for a tool name (`"agenda:list"` → `"agenda"`).
#[must_use]
pub fn tool_domain(name: &str) -> Option<&str> {
    let (prefix, rest) = name.split_once(':')?;
    if prefix.is_empty() || rest.is_empty() {
        return None;
    }
    Some(prefix)
}

/// Affinity bucket for margin multi-domain union.
///
/// Only domains that share a bucket are cluster-unioned on a near-tie. Unrelated
/// mush (e.g. `moltbook` + `web` + `db` + `doc`) stays a ranked subset — no A–Z dump.
#[must_use]
pub fn affinity_group(domain: &str) -> Option<&'static str> {
    match domain {
        "agenda" | "clock" | "calendar" => Some("time"),
        "web" | "news" | "wiki" => Some("web"),
        "doc" | "vault" | "memory" | "media" => Some("knowledge"),
        "mail" => Some("mail"),
        "weather" => Some("weather"),
        "moltbook" => Some("moltbook"),
        "db" => Some("db"),
        "vision" => Some("vision"),
        "system" | "skills" => Some("system"),
        _ => None,
    }
}

/// True when two domains may be widened together on a near-tie.
#[must_use]
pub fn domains_share_affinity(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (affinity_group(a), affinity_group(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Registered tools whose names start with `{domain}:`.
#[must_use]
pub fn cluster_members(domain: &str, registered: &[String]) -> Vec<String> {
    let needle = format!("{domain}:");
    let mut out: Vec<String> = registered
        .iter()
        .filter(|n| n.starts_with(&needle))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Union of cluster members for every distinct domain in `tool_names`.
///
/// Order is alphabetical for a stable set; callers that feed slim/GBNF should
/// re-rank by cosine (see [`super::policy`]).
#[must_use]
pub fn union_clusters_for_tools(tool_names: &[String], registered: &[String]) -> Vec<String> {
    let mut domains: Vec<&str> = tool_names.iter().filter_map(|n| tool_domain(n)).collect();
    domains.sort_unstable();
    domains.dedup();

    let mut out = Vec::new();
    for domain in domains {
        out.extend(cluster_members(domain, registered));
    }
    out.sort();
    out.dedup();
    out
}

/// Expand a set of tool names to include every registered sibling in the same domain(s).
pub fn expand_names_to_domain_clusters(
    names: impl IntoIterator<Item = String>,
    registered: &[String],
) -> Vec<String> {
    let seed: Vec<String> = names.into_iter().collect();
    if seed.is_empty() {
        return Vec::new();
    }
    union_clusters_for_tools(&seed, registered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_registry() -> Vec<String> {
        vec![
            "agenda:list".into(),
            "agenda:remove".into(),
            "agenda:complete".into(),
            "agenda:remind_at".into(),
            "clock:now".into(),
            "clock:alarm".into(),
            "clock:timer".into(),
            "doc:ingest".into(),
            "doc:query".into(),
            "web:fetch".into(),
            "web:find".into(),
            "web:search".into(),
        ]
    }

    #[test]
    fn tool_domain_parses_prefix() {
        assert_eq!(tool_domain("agenda:list"), Some("agenda"));
        assert_eq!(tool_domain("clock:alarm"), Some("clock"));
        assert_eq!(tool_domain("nope"), None);
        assert_eq!(tool_domain(":bad"), None);
    }

    #[test]
    fn cluster_members_filters_prefix() {
        let reg = sample_registry();
        let agenda = cluster_members("agenda", &reg);
        assert!(agenda.iter().all(|n| n.starts_with("agenda:")));
        assert!(agenda.contains(&"agenda:remove".to_string()));
        assert!(!agenda.iter().any(|n| n.starts_with("clock:")));
    }

    #[test]
    fn union_clusters_merges_clock_and_agenda() {
        let reg = sample_registry();
        let seeds = vec!["clock:alarm".into(), "agenda:remind_at".into()];
        let union = union_clusters_for_tools(&seeds, &reg);
        assert!(union.contains(&"clock:timer".to_string()));
        assert!(union.contains(&"agenda:list".to_string()));
        assert!(!union.contains(&"doc:ingest".to_string()));
    }

    #[test]
    fn affinity_groups_time_but_not_db_web() {
        assert!(domains_share_affinity("clock", "agenda"));
        assert!(domains_share_affinity("web", "news"));
        assert!(!domains_share_affinity("moltbook", "web"));
        assert!(!domains_share_affinity("db", "web"));
        assert_eq!(affinity_group("clock"), Some("time"));
    }
}
