//! Shared JSON shape for staged `web:fetch` payloads (`web:artifact_query` must deserialize the same).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WebOutboundLink {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_text: Option<String>,
    /// 1 = best heuristic rank (HTML navigational target, not asset URL).
    pub rank: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WebArtifact {
    pub url: String,
    pub chunks: Vec<String>,
    #[serde(default)]
    pub outbound_links: Vec<WebOutboundLink>,
}
