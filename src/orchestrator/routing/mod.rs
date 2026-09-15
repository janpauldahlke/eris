//! Pre-LLM tool-offer policy: signals → decision → slim/GBNF offer set.
//!
//! Phase 1 shipped demotion / affinity unions / dialog pairing.
//! Phase 2 formalizes [`RoutingOffer`] and named rule ids without changing
//! the orchestrator's slim/GBNF wiring conventions.

pub mod clusters;
pub mod decision;
pub mod dialog;
pub mod overlays;
pub mod policy;
pub mod signals;

pub use clusters::{
    affinity_group, cluster_members, domains_share_affinity, expand_names_to_domain_clusters,
    tool_domain, union_clusters_for_tools,
};
pub use decision::{RoutingDecision, RoutingOffer, UnsureFallback};
pub use overlays::apply_offer_overlays;
pub use policy::{
    RoutingPolicyKnobs, URL_SOFT_COMPEL_HINT, apply_routing_policy, decide,
    should_soft_compel_web_fetch, user_text_has_url,
};
pub use signals::{
    RoutingSignals, has_agenda_continuation_intent, has_doc_delete_continuation,
    has_doc_ingest_cues,
};
