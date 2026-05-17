//! Session-scoped anti-crawl ledger for `web:fetch` and internal `news:today` fetches.

use crate::config::WebConfig;
use crate::executive::error::{FcpError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use url::Url;

pub mod policy {
    pub const WEB_TURN_CAP: &str = "WEB_TURN_CAP";
    pub const WEB_SESSION_CAP: &str = "WEB_SESSION_CAP";
    pub const WEB_MISSION_BUDGET: &str = "WEB_MISSION_BUDGET";
    pub const WEB_FIND_BEFORE_REFETCH: &str = "WEB_FIND_BEFORE_REFETCH";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebUrlEntry {
    pub artifact_id: String,
    pub mission_id: String,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebMissionState {
    pub budget_max: u32,
    pub pages_used: u32,
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebCacheHit {
    pub normalized_url: String,
    pub artifact_id: String,
    pub mission_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchReservation {
    pub normalized_url: String,
    pub mission_id: String,
    pub artifact_id: String,
    pub budget_max: u32,
    pub budget_remaining_after: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WebSessionLedger {
    urls: HashMap<String, WebUrlEntry>,
    missions: HashMap<String, WebMissionState>,
    #[serde(default)]
    last_find_at_by_artifact: HashMap<String, DateTime<Utc>>,
    #[serde(default)]
    hosts_pending_find: HashSet<String>,
    fetches_this_turn: u32,
    fetches_this_session: u32,
}

impl WebSessionLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset_session(&mut self) {
        *self = Self::default();
    }

    pub fn begin_user_turn(&mut self) {
        self.fetches_this_turn = 0;
    }

    pub fn fetches_this_turn(&self) -> u32 {
        self.fetches_this_turn
    }

    pub fn fetches_this_session(&self) -> u32 {
        self.fetches_this_session
    }

    pub fn lookup_url(&self, normalized_url: &str) -> Option<&WebUrlEntry> {
        self.urls.get(normalized_url)
    }

    pub fn mission_id_for_artifact(&self, artifact_id: &str) -> Option<String> {
        self.urls
            .values()
            .find(|e| e.artifact_id == artifact_id)
            .map(|e| e.mission_id.clone())
    }

    pub fn mission_state(&self, mission_id: &str) -> Option<&WebMissionState> {
        self.missions.get(mission_id)
    }

    /// Clamp agent `fetch_budget` against config and optional continuing mission headroom.
    pub fn clamp_fetch_budget(
        request: Option<u32>,
        config: &WebConfig,
        mission_remaining: Option<u32>,
    ) -> u32 {
        let requested = request.unwrap_or(config.default_fetch_budget);
        let mut budget = requested
            .min(config.default_fetch_budget)
            .min(config.max_fetch_budget_override);
        if let Some(remaining) = mission_remaining {
            budget = budget.min(remaining);
        }
        budget.min(config.max_fetches_per_mission).max(1)
    }

    /// Pre-flight before browser39: caps, duplicate cache, find-before-refetch.
    pub fn reserve_fetch(
        &mut self,
        config: &WebConfig,
        raw_url: &str,
        mission_id: Option<&str>,
        fetch_budget: Option<u32>,
        new_artifact_id: &str,
        new_mission_id: &str,
    ) -> Result<std::result::Result<WebFetchReservation, WebCacheHit>> {
        let normalized_url = normalize_url(raw_url)?;

        if let Some(entry) = self.urls.get(&normalized_url) {
            return Ok(Err(WebCacheHit {
                normalized_url,
                artifact_id: entry.artifact_id.clone(),
                mission_id: entry.mission_id.clone(),
            }));
        }

        self.enforce_turn_and_session_caps(config)?;

        let host = host_from_normalized_url(&normalized_url).ok_or_else(|| {
            FcpError::SchemaViolation(format!("web: URL has no host: {raw_url}"))
        })?;

        let continuing_mission = mission_id
            .filter(|s| !s.trim().is_empty())
            .is_some_and(|mid| self.missions.contains_key(mid.trim()));

        if config.require_find_before_refetch
            && !continuing_mission
            && self.hosts_pending_find.contains(&host)
        {
            let hint = self
                .hosts_pending_find_hint(&host)
                .unwrap_or_else(|| "run web:find on a page from this host first".to_string());
            return Err(policy_error(
                policy::WEB_FIND_BEFORE_REFETCH,
                format!("web:fetch blocked for host `{host}`: {hint}"),
            ));
        }

        let (mission_id, budget_max, budget_remaining_after) =
            if let Some(mid) = mission_id.filter(|s| !s.trim().is_empty()) {
                let mid = mid.trim().to_string();
                let mission = self.missions.get(&mid).ok_or_else(|| {
                    FcpError::ToolFault {
                        tool_name: "web:fetch".into(),
                        reason: format!(
                            "unknown mission_id `{mid}` — omit mission_id to start a new mission, or pass an id from a prior fetch receipt"
                        ),
                    }
                })?;
                if mission.pages_used >= mission.budget_max {
                    return Err(policy_error(
                        policy::WEB_MISSION_BUDGET,
                        format!(
                            "mission `{mid}` fetch budget exhausted ({}/{})",
                            mission.pages_used, mission.budget_max
                        ),
                    ));
                }
                let remaining = mission.budget_max.saturating_sub(mission.pages_used);
                let _budget_clamp = Self::clamp_fetch_budget(fetch_budget, config, Some(remaining));
                if mission.pages_used + 1 > mission.budget_max {
                    return Err(policy_error(
                        policy::WEB_MISSION_BUDGET,
                        format!(
                            "mission `{mid}` cannot accept another fetch ({}/{})",
                            mission.pages_used, mission.budget_max
                        ),
                    ));
                }
                let after = remaining.saturating_sub(1);
                (mid, mission.budget_max, after)
            } else {
                let budget_max =
                    Self::clamp_fetch_budget(fetch_budget, config, None);
                (
                    new_mission_id.to_string(),
                    budget_max,
                    budget_max.saturating_sub(1),
                )
            };

        if self.missions.get(&mission_id).is_none() {
            self.missions.insert(
                mission_id.clone(),
                WebMissionState {
                    budget_max,
                    pages_used: 0,
                    urls: Vec::new(),
                },
            );
        }

        Ok(Ok(WebFetchReservation {
            normalized_url,
            mission_id,
            artifact_id: new_artifact_id.to_string(),
            budget_max,
            budget_remaining_after,
        }))
    }

    /// Record a successful fetch (call after vault page write).
    pub fn commit_fetch(
        &mut self,
        normalized_url: String,
        artifact_id: String,
        mission_id: String,
        host: String,
    ) {
        let now = Utc::now();
        self.urls.insert(
            normalized_url.clone(),
            WebUrlEntry {
                artifact_id: artifact_id.clone(),
                mission_id: mission_id.clone(),
                fetched_at: now,
            },
        );
        if let Some(mission) = self.missions.get_mut(&mission_id) {
            mission.pages_used = mission.pages_used.saturating_add(1);
            if !mission.urls.contains(&normalized_url) {
                mission.urls.push(normalized_url);
            }
        }
        self.hosts_pending_find.insert(host);
        self.fetches_this_turn = self.fetches_this_turn.saturating_add(1);
        self.fetches_this_session = self.fetches_this_session.saturating_add(1);
    }

    pub fn record_find(&mut self, artifact_id: &str) {
        let now = Utc::now();
        self.last_find_at_by_artifact
            .insert(artifact_id.to_string(), now);
        if let Some(host) = self.host_for_artifact(artifact_id) {
            self.hosts_pending_find.remove(&host);
        }
    }

    pub fn persist_path(vault_root: &Path) -> std::path::PathBuf {
        vault_root.join(".fcp/web_session.json")
    }

    pub fn load_from_vault(vault_root: &Path, config: &WebConfig) -> Result<Self> {
        if !config.persist_ledger {
            return Ok(Self::new());
        }
        let path = Self::persist_path(vault_root);
        if !path.is_file() {
            return Ok(Self::new());
        }
        let bytes = std::fs::read(&path).map_err(FcpError::Io)?;
        let ledger: Self = serde_json::from_slice(&bytes).map_err(FcpError::ParseFault)?;
        Ok(ledger)
    }

    pub fn save_to_vault(&self, vault_root: &Path, config: &WebConfig) -> Result<()> {
        if !config.persist_ledger {
            return Ok(());
        }
        let path = Self::persist_path(vault_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(FcpError::Io)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(FcpError::ParseFault)?;
        std::fs::write(path, bytes).map_err(FcpError::Io)
    }

    fn enforce_turn_and_session_caps(&self, config: &WebConfig) -> Result<()> {
        if self.fetches_this_turn >= config.max_fetches_per_user_turn {
            return Err(policy_error(
                policy::WEB_TURN_CAP,
                format!(
                    "web fetch turn cap reached ({}/{})",
                    self.fetches_this_turn, config.max_fetches_per_user_turn
                ),
            ));
        }
        if self.fetches_this_session >= config.max_fetches_per_chat_session {
            return Err(policy_error(
                policy::WEB_SESSION_CAP,
                format!(
                    "web fetch session cap reached ({}/{})",
                    self.fetches_this_session, config.max_fetches_per_chat_session
                ),
            ));
        }
        Ok(())
    }

    fn host_for_artifact(&self, artifact_id: &str) -> Option<String> {
        self.urls.iter().find_map(|(url, entry)| {
            if entry.artifact_id == artifact_id {
                host_from_normalized_url(url)
            } else {
                None
            }
        })
    }

    fn hosts_pending_find_hint(&self, host: &str) -> Option<String> {
        let artifact = self.urls.iter().find_map(|(url, entry)| {
            host_from_normalized_url(url)
                .filter(|h| h == host)
                .map(|_| entry.artifact_id.clone())
        })?;
        Some(format!(
            "try web:find on artifact_id `{artifact}` before fetching again"
        ))
    }
}

fn policy_error(code: &str, message: String) -> FcpError {
    FcpError::PolicyViolation {
        code: code.to_string(),
        message,
    }
}

/// Dedup key: `scheme + host + path` only (no query, no fragment).
pub fn normalize_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let parsed = Url::parse(trimmed).map_err(|e| {
        FcpError::SchemaViolation(format!("web: invalid URL ({trimmed}): {e}"))
    })?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(FcpError::SchemaViolation(format!(
            "web: URL scheme must be http or https: {trimmed}"
        )));
    }
    let host = parsed.host_str().ok_or_else(|| {
        FcpError::SchemaViolation(format!("web: URL has no host: {trimmed}"))
    })?;
    let path = parsed.path();
    let path = if path.is_empty() { "/" } else { path };
    Ok(format!("{scheme}://{host}{path}"))
}

/// Host equality for find-before-refetch and same-host link rules (Q6).
pub fn normalize_host(host: &str) -> String {
    let h = host.trim().to_lowercase();
    h.strip_prefix("www.")
        .map(str::to_string)
        .unwrap_or(h)
}

pub fn host_from_normalized_url(normalized: &str) -> Option<String> {
    let parsed = Url::parse(normalized).ok()?;
    parsed.host_str().map(normalize_host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_config() -> WebConfig {
        WebConfig::default()
    }

    #[test]
    fn normalize_url_strips_query_and_fragment() {
        let n = normalize_url("https://WWW.Example.com/path?q=1&x=2#frag").expect("ok");
        assert_eq!(n, "https://www.example.com/path");
    }

    #[test]
    fn normalize_host_strips_www() {
        assert_eq!(normalize_host("WWW.BBC.com"), "bbc.com");
        assert_eq!(normalize_host("bbc.com"), "bbc.com");
    }

    #[test]
    fn reserve_without_mission_id_registers_new_mission() {
        let mut ledger = WebSessionLedger::new();
        let config = test_config();
        let res = ledger
            .reserve_fetch(
                &config,
                "https://example.com/home",
                None,
                Some(3),
                "art-new",
                "mis-new",
            )
            .expect("ok")
            .expect("reserve");
        assert!(!res.mission_id.is_empty());
        assert!(ledger.missions.contains_key(&res.mission_id));
    }

    #[test]
    fn reserve_unknown_mission_id_is_tool_fault_not_schema() {
        let mut ledger = WebSessionLedger::new();
        let err = ledger
            .reserve_fetch(
                &test_config(),
                "https://example.com/x",
                Some("does-not-exist"),
                None,
                "art",
                "mis-fallback",
            )
            .expect_err("unknown mission");
        match err {
            FcpError::ToolFault { reason, .. } => {
                assert!(reason.contains("unknown mission_id"));
            }
            other => panic!("expected ToolFault, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_url_returns_cache_hit_without_consuming_budget() {
        let mut ledger = WebSessionLedger::new();
        let config = test_config();
        let url = "https://example.com/a";
        let norm = normalize_url(url).expect("norm");
        ledger.commit_fetch(
            norm.clone(),
            "art-1".into(),
            "mis-1".into(),
            "example.com".into(),
        );
        let hit = ledger
            .reserve_fetch(
                &config,
                url,
                None,
                Some(2),
                "art-2",
                "mis-2",
            )
            .expect("ok")
            .expect_err("cached");
        assert_eq!(hit.artifact_id, "art-1");
        assert_eq!(hit.mission_id, "mis-1");
        assert_eq!(ledger.fetches_this_session(), 1);
    }

    #[test]
    fn turn_cap_returns_web_turn_cap() {
        let mut ledger = WebSessionLedger::new();
        let mut config = test_config();
        config.max_fetches_per_user_turn = 1;
        ledger.fetches_this_turn = 1;
        let err = ledger
            .reserve_fetch(
                &config,
                "https://example.com/one",
                None,
                None,
                "a",
                "m",
            )
            .expect_err("cap");
        match err {
            FcpError::PolicyViolation { code, .. } => assert_eq!(code, policy::WEB_TURN_CAP),
            other => panic!("expected PolicyViolation, got {other:?}"),
        }
    }

    #[test]
    fn session_cap_returns_web_session_cap() {
        let mut ledger = WebSessionLedger::new();
        let mut config = test_config();
        config.max_fetches_per_chat_session = 2;
        ledger.fetches_this_session = 2;
        let err = ledger
            .reserve_fetch(
                &config,
                "https://example.com/two",
                None,
                None,
                "a",
                "m",
            )
            .expect_err("cap");
        match err {
            FcpError::PolicyViolation { code, .. } => assert_eq!(code, policy::WEB_SESSION_CAP),
            other => panic!("expected PolicyViolation, got {other:?}"),
        }
    }

    #[test]
    fn mission_budget_exhausted_returns_web_mission_budget() {
        let mut ledger = WebSessionLedger::new();
        let config = test_config();
        let mid = Uuid::new_v4().to_string();
        ledger.missions.insert(
            mid.clone(),
            WebMissionState {
                budget_max: 1,
                pages_used: 1,
                urls: vec!["https://example.com/".into()],
            },
        );
        let err = ledger
            .reserve_fetch(
                &config,
                "https://example.com/other",
                Some(&mid),
                None,
                "a",
                "m",
            )
            .expect_err("budget");
        match err {
            FcpError::PolicyViolation { code, .. } => assert_eq!(code, policy::WEB_MISSION_BUDGET),
            other => panic!("expected PolicyViolation, got {other:?}"),
        }
    }

    #[test]
    fn find_before_refetch_blocks_same_host() {
        let mut ledger = WebSessionLedger::new();
        let config = test_config();
        ledger.commit_fetch(
            "https://www.example.com/page".into(),
            "art-1".into(),
            "mis-1".into(),
            "example.com".into(),
        );
        let err = ledger
            .reserve_fetch(
                &config,
                "https://example.com/other",
                None,
                None,
                "art-2",
                "mis-2",
            )
            .expect_err("find first");
        match err {
            FcpError::PolicyViolation { code, message } => {
                assert_eq!(code, policy::WEB_FIND_BEFORE_REFETCH);
                assert!(message.contains("art-1"));
            }
            other => panic!("expected PolicyViolation, got {other:?}"),
        }
    }

    #[test]
    fn continuing_mission_skips_find_before_refetch() {
        let mut ledger = WebSessionLedger::new();
        let config = test_config();
        let mid = Uuid::new_v4().to_string();
        ledger.missions.insert(
            mid.clone(),
            WebMissionState {
                budget_max: 4,
                pages_used: 1,
                urls: vec!["https://www.bbc.com/".into()],
            },
        );
        ledger.commit_fetch(
            "https://www.bbc.com/".into(),
            "art-home".into(),
            mid.clone(),
            "bbc.com".into(),
        );
        let res = ledger
            .reserve_fetch(
                &config,
                "https://www.bbc.com/news/world-123",
                Some(&mid),
                None,
                "art-deep",
                "mis-unused",
            )
            .expect("ok")
            .expect("reserved");
        assert_eq!(res.artifact_id, "art-deep");
        assert_eq!(res.mission_id, mid);
    }

    #[test]
    fn record_find_clears_host_gate() {
        let mut ledger = WebSessionLedger::new();
        let config = test_config();
        ledger.commit_fetch(
            "https://example.com/a".into(),
            "art-1".into(),
            "mis-1".into(),
            "example.com".into(),
        );
        ledger.record_find("art-1");
        let res = ledger
            .reserve_fetch(
                &config,
                "https://www.example.com/b",
                None,
                None,
                "art-2",
                &Uuid::new_v4().to_string(),
            )
            .expect("ok")
            .expect("reserved");
        assert_eq!(res.artifact_id, "art-2");
    }

    #[test]
    fn persist_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = test_config();
        config.persist_ledger = true;
        let mut ledger = WebSessionLedger::new();
        ledger.commit_fetch(
            "https://example.com/x".into(),
            "art-p".into(),
            "mis-p".into(),
            "example.com".into(),
        );
        ledger.save_to_vault(dir.path(), &config).expect("save");
        let loaded = WebSessionLedger::load_from_vault(dir.path(), &config).expect("load");
        assert_eq!(loaded.fetches_this_session(), 1);
        assert!(loaded.lookup_url("https://example.com/x").is_some());
    }
}
