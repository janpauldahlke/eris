use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Ephemeral tier for staged memory rows. Determines TTL bucket and commit eligibility.
///
/// Promotion order: `Session` -> `Scratch` -> `Promote`.
/// Only `Promote` is eligible for `memory:commit_all`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralTier {
    #[default]
    Session,
    Scratch,
    Promote,
}

impl EphemeralTier {
    /// Next tier in the promotion ladder, if any.
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Session => Some(Self::Scratch),
            Self::Scratch => Some(Self::Promote),
            Self::Promote => None,
        }
    }

    /// Previous tier (for decay/downgrade).
    pub fn prev(self) -> Option<Self> {
        match self {
            Self::Session => None,
            Self::Scratch => Some(Self::Session),
            Self::Promote => Some(Self::Scratch),
        }
    }

    /// Numeric index (0-based) for score threshold lookups.
    pub fn index(self) -> usize {
        match self {
            Self::Session => 0,
            Self::Scratch => 1,
            Self::Promote => 2,
        }
    }
}

impl fmt::Display for EphemeralTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session => write!(f, "session"),
            Self::Scratch => write!(f, "scratch"),
            Self::Promote => write!(f, "promote"),
        }
    }
}

/// Vault root category for committed memory. Determines which top-level directory
/// receives the written markdown file.
///
/// - `Topology`: environment, config, infrastructure (`10_Topology/`)
/// - `Discourse`: raw interaction, append-only stream (`20_Discourse/`)
/// - `Synthesis`: zettelkasten nodes, revisioned atomic concepts (`30_Synthesis/`)
///
/// `00_Invariants/` is never a valid write target for the agent.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    Topology,
    Discourse,
    #[default]
    Synthesis,
}

impl VaultKind {
    /// The vault directory name for this kind.
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Topology => "10_Topology",
            Self::Discourse => "20_Discourse",
            Self::Synthesis => "30_Synthesis",
        }
    }
}

impl fmt::Display for VaultKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Topology => write!(f, "topology"),
            Self::Discourse => write!(f, "discourse"),
            Self::Synthesis => write!(f, "synthesis"),
        }
    }
}

/// Epistemic status for committed vault notes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicStatus {
    #[default]
    Candidate,
    Stable,
    Contested,
    Deprecated,
    Retracted,
}

impl fmt::Display for EpistemicStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Candidate => write!(f, "candidate"),
            Self::Stable => write!(f, "stable"),
            Self::Contested => write!(f, "contested"),
            Self::Deprecated => write!(f, "deprecated"),
            Self::Retracted => write!(f, "retracted"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_promotion_ladder() {
        assert_eq!(EphemeralTier::Session.next(), Some(EphemeralTier::Scratch));
        assert_eq!(EphemeralTier::Scratch.next(), Some(EphemeralTier::Promote));
        assert_eq!(EphemeralTier::Promote.next(), None);
    }

    #[test]
    fn tier_demotion_ladder() {
        assert_eq!(EphemeralTier::Promote.prev(), Some(EphemeralTier::Scratch));
        assert_eq!(EphemeralTier::Scratch.prev(), Some(EphemeralTier::Session));
        assert_eq!(EphemeralTier::Session.prev(), None);
    }

    #[test]
    fn tier_ordering_is_promotion_order() {
        assert!(EphemeralTier::Session < EphemeralTier::Scratch);
        assert!(EphemeralTier::Scratch < EphemeralTier::Promote);
    }

    #[test]
    fn tier_default_is_session() {
        assert_eq!(EphemeralTier::default(), EphemeralTier::Session);
    }

    #[test]
    fn vault_kind_dir_names() {
        assert_eq!(VaultKind::Topology.dir_name(), "10_Topology");
        assert_eq!(VaultKind::Discourse.dir_name(), "20_Discourse");
        assert_eq!(VaultKind::Synthesis.dir_name(), "30_Synthesis");
    }

    #[test]
    fn vault_kind_default_is_synthesis() {
        assert_eq!(VaultKind::default(), VaultKind::Synthesis);
    }

    #[test]
    fn epistemic_status_default_is_candidate() {
        assert_eq!(EpistemicStatus::default(), EpistemicStatus::Candidate);
    }

    #[test]
    fn tier_serde_roundtrip() {
        let json = serde_json::to_string(&EphemeralTier::Promote).expect("serialize");
        assert_eq!(json, "\"promote\"");
        let back: EphemeralTier = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, EphemeralTier::Promote);
    }

    #[test]
    fn vault_kind_serde_roundtrip() {
        let json = serde_json::to_string(&VaultKind::Discourse).expect("serialize");
        assert_eq!(json, "\"discourse\"");
        let back: VaultKind = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, VaultKind::Discourse);
    }

    #[test]
    fn epistemic_status_serde_roundtrip() {
        let json = serde_json::to_string(&EpistemicStatus::Contested).expect("serialize");
        assert_eq!(json, "\"contested\"");
        let back: EpistemicStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, EpistemicStatus::Contested);
    }
}
