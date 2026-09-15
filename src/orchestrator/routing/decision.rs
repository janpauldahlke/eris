//! Explicit pre-LLM routing offer (Phase 2).
//!
//! Cosine hits and lexical guards remain signals; this type is the **decision**
//! that drives slim prompt assembly and GBNF subset selection.

/// What the orchestrator should offer the model this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingOffer {
    /// No tool schemas; conversational envelope only.
    Conversational,
    /// Named tools (cosine-ranked). May be a tight hit list or an expanded set.
    Subset(Vec<String>),
    /// Affinity / dialog cluster expansion. `domains` is for telemetry; `tools` is the offer.
    DomainCluster {
        domains: Vec<&'static str>,
        tools: Vec<String>,
    },
    /// Tool mode with empty name list → full allowed roster (existing slim/GBNF convention).
    FullRoster,
}

impl RoutingOffer {
    #[must_use]
    pub fn tools_needed(&self) -> bool {
        !matches!(self, Self::Conversational)
    }

    /// Names for slim map / GBNF. Empty means full roster when [`Self::tools_needed`].
    #[must_use]
    pub fn matched_tool_names(&self) -> Vec<String> {
        match self {
            Self::Conversational | Self::FullRoster => Vec::new(),
            Self::Subset(tools) => tools.clone(),
            Self::DomainCluster { tools, .. } => tools.clone(),
        }
    }

    #[must_use]
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Conversational => "conversational",
            Self::Subset(_) => "subset",
            Self::DomainCluster { .. } => "domain_cluster",
            Self::FullRoster => "full_roster",
        }
    }
}

/// Full pre-LLM routing outcome: offer + which rule produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    pub offer: RoutingOffer,
    /// Stable rule id for logs (`AGENDA_DIALOG_PAIRING`, `SHORT_INPUT`, …).
    pub rule_id: &'static str,
}

impl RoutingDecision {
    #[must_use]
    pub fn conversational(rule_id: &'static str) -> Self {
        Self {
            offer: RoutingOffer::Conversational,
            rule_id,
        }
    }

    #[must_use]
    pub fn full_roster(rule_id: &'static str) -> Self {
        Self {
            offer: RoutingOffer::FullRoster,
            rule_id,
        }
    }

    #[must_use]
    pub fn subset(rule_id: &'static str, tools: Vec<String>) -> Self {
        Self {
            offer: RoutingOffer::Subset(tools),
            rule_id,
        }
    }

    #[must_use]
    pub fn domain_cluster(
        rule_id: &'static str,
        domains: Vec<&'static str>,
        tools: Vec<String>,
    ) -> Self {
        Self {
            offer: RoutingOffer::DomainCluster { domains, tools },
            rule_id,
        }
    }

    #[must_use]
    pub fn tools_needed(&self) -> bool {
        self.offer.tools_needed()
    }

    #[must_use]
    pub fn matched_tool_names(&self) -> Vec<String> {
        self.offer.matched_tool_names()
    }
}

/// When a lone weak embed hit is demoted, how to fall through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnsureFallback {
    /// Empty matched names → full allowed roster (historic default).
    #[default]
    FullRoster,
    /// Expand the demoted tool's domain cluster instead of opening the full roster.
    DomainCluster,
}

impl UnsureFallback {
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "domain_cluster" | "cluster" => Self::DomainCluster,
            _ => Self::FullRoster,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullRoster => "full_roster",
            Self::DomainCluster => "domain_cluster",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_roster_and_conversational_expose_empty_names() {
        assert!(RoutingOffer::FullRoster.tools_needed());
        assert!(RoutingOffer::FullRoster.matched_tool_names().is_empty());
        assert!(!RoutingOffer::Conversational.tools_needed());
    }

    #[test]
    fn unsure_fallback_parse() {
        assert_eq!(
            UnsureFallback::parse("domain_cluster"),
            UnsureFallback::DomainCluster
        );
        assert_eq!(
            UnsureFallback::parse("full_roster"),
            UnsureFallback::FullRoster
        );
        assert_eq!(UnsureFallback::parse("nope"), UnsureFallback::FullRoster);
    }
}
