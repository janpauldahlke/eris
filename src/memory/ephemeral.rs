use crate::executive::error::Result;
use crate::memory::types::{EphemeralTier, VaultKind};
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheValue {
    pub staged_id: String,
    pub title: String,
    pub data: String,
    pub tags: Vec<String>,
    pub expires_at: u64, // Absolute UNIX timestamp
    /// Stable UUID for this concept across turns and commits.
    #[serde(default = "default_node_id")]
    pub node_id: String,
    /// Normalized form of the title (NFKC, lowercase, slug). Primary dedupe key.
    #[serde(default)]
    pub canonical_key: String,
    /// Current ephemeral tier. Determines TTL bucket and commit eligibility.
    #[serde(default)]
    pub tier: EphemeralTier,
    /// Numeric score driving tier promotion. Incremented by mentions and explicit staging.
    #[serde(default)]
    pub promotion_score: f64,
    /// Number of distinct turns this concept has appeared in.
    #[serde(default)]
    pub mention_count: u32,
    /// When true, this entry has a detected contradiction and should not auto-promote.
    #[serde(default)]
    pub needs_review: bool,
    /// UNIX timestamp of first appearance.
    #[serde(default)]
    pub first_seen_at: u64,
    /// UNIX timestamp of most recent mention/update.
    #[serde(default)]
    pub last_seen_at: u64,
    /// Vault root category hint for commit routing. Default: `Synthesis`.
    #[serde(default)]
    pub kind: VaultKind,
}

fn default_node_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub struct EphemeralMemory {
    pub cache: Cache<String, CacheValue>,
    pub workspace: String,
}

impl EphemeralMemory {
    pub fn new(workspace: String) -> Self {
        let cache = Cache::builder().max_capacity(10_000).build();

        Self { cache, workspace }
    }

    pub async fn insert(
        &self,
        title: &str,
        value: &str,
        tags: Vec<String>,
        ttl_secs: u64,
    ) -> Result<CacheValue> {
        self.insert_with_tier(
            title,
            value,
            tags,
            ttl_secs,
            EphemeralTier::Session,
            VaultKind::default(),
        )
        .await
    }

    pub async fn insert_with_tier(
        &self,
        title: &str,
        value: &str,
        tags: Vec<String>,
        ttl_secs: u64,
        tier: EphemeralTier,
        kind: VaultKind,
    ) -> Result<CacheValue> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let now_secs = now.as_secs();
        let expires_at = now_secs + ttl_secs;
        let staged_id = uuid::Uuid::new_v4().to_string();
        let node_id = uuid::Uuid::new_v4().to_string();
        let canonical_key = normalize_canonical_key(title);

        let cache_value = CacheValue {
            staged_id: staged_id.clone(),
            title: title.to_string(),
            data: value.to_string(),
            tags,
            expires_at,
            node_id,
            canonical_key,
            tier,
            promotion_score: 0.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: now_secs,
            last_seen_at: now_secs,
            kind,
        };

        self.cache.insert(staged_id, cache_value.clone()).await;
        tracing::debug!(
            staged_id = %cache_value.staged_id,
            title = %cache_value.title,
            node_id = %cache_value.node_id,
            canonical_key = %cache_value.canonical_key,
            tier = %cache_value.tier,
            kind = %cache_value.kind,
            ttl_secs,
            "Ephemeral insert"
        );
        Ok(cache_value)
    }

    fn is_expired(expires_at: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now >= expires_at
    }

    pub async fn get_by_id(&self, staged_id: &str) -> Option<CacheValue> {
        let val = self.cache.get(staged_id).await?;
        if Self::is_expired(val.expires_at) {
            self.cache.invalidate(staged_id).await;
            return None;
        }
        Some(val)
    }

    pub async fn get_by_title(&self, title: &str) -> Option<CacheValue> {
        for (id, entry) in self.cache.iter() {
            if entry.title == title {
                if Self::is_expired(entry.expires_at) {
                    self.cache.invalidate(id.as_str()).await;
                    continue;
                }
                return Some(entry);
            }
        }
        None
    }

    /// Removes all cache rows with this title (e.g. before replacing with [`Self::upsert_by_title`]).
    pub async fn invalidate_by_title(&self, title: &str) {
        let keys: Vec<String> = self
            .cache
            .iter()
            .filter(|(_, v)| v.title == title)
            .map(|(k, _)| k.to_string())
            .collect();
        for k in keys {
            self.cache.invalidate(&k).await;
        }
    }

    /// Replaces any existing entry with `title` by inserting a fresh value.
    /// Preserves `node_id` from a prior entry with the same canonical_key if one exists.
    pub async fn upsert_by_title(
        &self,
        title: &str,
        value: &str,
        tags: Vec<String>,
        ttl_secs: u64,
    ) -> Result<CacheValue> {
        let prior = self
            .get_by_canonical_key(&normalize_canonical_key(title))
            .await;
        self.invalidate_by_title(title).await;
        let mut new_val = self.insert(title, value, tags, ttl_secs).await?;
        if let Some(prev) = prior {
            // Preserve stable identity across upserts
            let staged_id = new_val.staged_id.clone();
            new_val.node_id = prev.node_id;
            new_val.first_seen_at = prev.first_seen_at;
            new_val.mention_count = prev.mention_count.saturating_add(1);
            new_val.promotion_score = prev.promotion_score;
            new_val.tier = prev.tier;
            new_val.needs_review = prev.needs_review;
            new_val.kind = prev.kind;
            self.cache.insert(staged_id, new_val.clone()).await;
        }
        Ok(new_val)
    }

    /// Set `needs_review` on an entry by `staged_id`. Returns `true` if found and updated.
    pub async fn set_needs_review(&self, staged_id: &str, needs_review: bool) -> bool {
        if let Some(mut val) = self.cache.get(staged_id).await {
            if val.needs_review != needs_review {
                val.needs_review = needs_review;
                self.cache.insert(staged_id.to_string(), val).await;
                tracing::info!(
                    staged_id = %staged_id,
                    needs_review,
                    "Ephemeral entry needs_review updated"
                );
            }
            true
        } else {
            false
        }
    }

    /// Find an entry by its normalized `canonical_key`.
    pub async fn get_by_canonical_key(&self, canonical_key: &str) -> Option<CacheValue> {
        for (id, entry) in self.cache.iter() {
            if entry.canonical_key == canonical_key {
                if Self::is_expired(entry.expires_at) {
                    self.cache.invalidate(id.as_str()).await;
                    continue;
                }
                return Some(entry);
            }
        }
        None
    }

    /// Find an entry by `node_id`.
    pub async fn get_by_node_id(&self, node_id: &str) -> Option<CacheValue> {
        for (id, entry) in self.cache.iter() {
            if entry.node_id == node_id {
                if Self::is_expired(entry.expires_at) {
                    self.cache.invalidate(id.as_str()).await;
                    continue;
                }
                return Some(entry);
            }
        }
        None
    }

    pub fn list_entries(&self) -> Vec<CacheValue> {
        self.cache
            .iter()
            .filter_map(|(_, v)| (!Self::is_expired(v.expires_at)).then_some(v.clone()))
            .collect()
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        self.get_by_title(key).await.map(|v| v.data)
    }

    pub fn collect_all_entries(&self) -> Vec<CacheValue> {
        self.list_entries()
    }

    pub fn collect_expired_ids(&self) -> Vec<String> {
        self.cache
            .iter()
            .filter(|(_, v)| Self::is_expired(v.expires_at))
            .map(|(k, _)| k.to_string())
            .collect()
    }

    pub fn collect_expired_entries(&self) -> Vec<CacheValue> {
        self.cache
            .iter()
            .filter_map(|(_, v)| Self::is_expired(v.expires_at).then_some(v.clone()))
            .collect()
    }

    pub async fn snapshot_to_disk(&self, vault_root: &std::path::Path) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entries: Vec<CacheValue> = self
            .cache
            .iter()
            .filter_map(|(_, v)| (v.expires_at > now).then_some(v.clone()))
            .collect();

        let path = crate::vault_layout::ephemeral_bin(vault_root, &self.workspace);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                crate::executive::error::FcpError::WorkspaceFault {
                    workspace: self.workspace.clone(),
                    reason: e.to_string(),
                }
            })?;
        }

        let ws = self.workspace.clone();
        let serialized = tokio::task::spawn_blocking(move || bincode::serialize(&entries))
            .await
            .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                workspace: ws.clone(),
                reason: e.to_string(),
            })?
            .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                workspace: ws,
                reason: e.to_string(),
            })?;

        tokio::fs::write(path, serialized).await.map_err(|e| {
            crate::executive::error::FcpError::WorkspaceFault {
                workspace: self.workspace.clone(),
                reason: e.to_string(),
            }
        })?;

        Ok(())
    }

    pub async fn load_from_disk(
        workspace: &str,
        vault_root: &std::path::Path,
        max_capacity: u64,
    ) -> Result<Self> {
        let cache = Cache::builder().max_capacity(max_capacity).build();

        let path = crate::vault_layout::ephemeral_bin(vault_root, workspace);

        if path.exists() {
            let data = tokio::fs::read(path).await.map_err(|e| {
                crate::executive::error::FcpError::WorkspaceFault {
                    workspace: workspace.to_string(),
                    reason: e.to_string(),
                }
            })?;
            if !data.is_empty() {
                let ws = workspace.to_string();
                let entries: Vec<CacheValue> =
                    tokio::task::spawn_blocking(move || bincode::deserialize(&data))
                        .await
                        .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                            workspace: ws.clone(),
                            reason: e.to_string(),
                        })?
                        .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                            workspace: ws,
                            reason: e.to_string(),
                        })?;

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                for v in entries {
                    if v.expires_at > now {
                        cache.insert(v.staged_id.clone(), v).await;
                    }
                }
            }
        }

        Ok(Self {
            cache,
            workspace: workspace.to_string(),
        })
    }
}

/// NFKC-normalize, lowercase, collapse non-alphanumeric runs to `_`, trim edges.
/// This is the authoritative canonical_key for ephemeral dedupe.
pub fn normalize_canonical_key(title: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let nfkc: String = title.nfkc().collect();
    let lower = nfkc.to_lowercase();
    let mut slug = String::with_capacity(lower.len());
    let mut last_was_sep = true; // suppress leading separator
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch.is_alphanumeric() {
            slug.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('_');
            last_was_sep = true;
        }
    }
    // Trim trailing separator
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

/// Staged rows from `web:fetch` must not be promoted to vault markdown (bloated HTML/JSON).
/// They are indexed in Qdrant at fetch time; `memory:commit` treats them as semantic-only.
pub fn is_web_artifact_staging(tags: &[String], title: &str) -> bool {
    tags.iter().any(|t| t == "web_artifact") || title.starts_with("web_artifact:")
}

/// Evaluate tier transitions and apply decay for all live ephemeral entries.
/// Called by the snapshot daemon on each tick.
pub async fn evaluate_promotions_and_decay(
    memory: &EphemeralMemory,
    config: &crate::config::AppConfig,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entries: Vec<CacheValue> = memory
        .cache
        .iter()
        .filter_map(|(_, v)| (!EphemeralMemory::is_expired(v.expires_at)).then_some(v.clone()))
        .collect();

    let mut promoted = 0u32;
    let mut demoted = 0u32;

    for entry in entries {
        let mut updated = entry.clone();

        // Apply decay
        updated.promotion_score =
            (updated.promotion_score - config.promotion_decay_per_tick).max(0.0);

        // Check upward promotion
        if let Some(threshold) = config.promotion_threshold_for_tier(updated.tier)
            && updated.promotion_score >= threshold
            && !updated.needs_review
            && let Some(next_tier) = updated.tier.next()
        {
            updated.tier = next_tier;
            let new_ttl = config.ttl_for_tier(next_tier);
            updated.expires_at = now + new_ttl;
            promoted += 1;
            tracing::debug!(
                node_id = %updated.node_id,
                title = %updated.title,
                new_tier = %next_tier,
                score = updated.promotion_score,
                "Tier promotion"
            );
        }

        // Check downward demotion (decay-driven)
        if let Some(prev_tier) = updated.tier.prev() {
            // Demotion: if score dropped below the threshold that got us here
            let threshold_to_current = config
                .promotion_threshold_for_tier(prev_tier)
                .unwrap_or(0.0);
            if updated.promotion_score < threshold_to_current * 0.5 {
                updated.tier = prev_tier;
                let new_ttl = config.ttl_for_tier(prev_tier);
                updated.expires_at = now + new_ttl;
                demoted += 1;
                tracing::debug!(
                    node_id = %updated.node_id,
                    title = %updated.title,
                    new_tier = %prev_tier,
                    score = updated.promotion_score,
                    "Tier demotion (decay)"
                );
            }
        }

        // Write back if changed
        if updated.tier != entry.tier
            || (updated.promotion_score - entry.promotion_score).abs() > f64::EPSILON
        {
            memory.cache.insert(entry.staged_id.clone(), updated).await;
        }
    }

    if promoted > 0 || demoted > 0 {
        tracing::info!(
            promoted,
            demoted,
            "Promotion daemon tick: tier transitions applied"
        );
    }
}

/// When `true`, the orchestrator is inside [`crate::orchestrator::core::Orchestrator::step`]; the
/// snapshot daemon skips [`evaluate_promotions_and_decay`] so decay/tier moves do not race slow LLM
/// or tool batches. Snapshot + expiry handling still run on their timers.
pub fn spawn_snapshot_daemon(
    memory: Arc<EphemeralMemory>,
    vault_root: PathBuf,
    _semantic: Option<Arc<crate::memory::semantic::SemanticBrain>>,
    interval_secs: u64,
    cancel_token: CancellationToken,
    config: Arc<crate::config::AppConfig>,
    promotion_suppressed_during_step: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    tokio::spawn(async move {
        let snapshot_interval = std::time::Duration::from_secs(interval_secs);
        let promo_interval = std::time::Duration::from_secs(config.promotion_eval_interval_secs);
        let mut snapshot_tick = tokio::time::interval(snapshot_interval);
        let mut promo_tick = tokio::time::interval(promo_interval);
        snapshot_tick.tick().await;
        promo_tick.tick().await;

        loop {
            tokio::select! {
                _ = snapshot_tick.tick() => {
                    let expired_entries = memory.collect_expired_entries();
                    for entry in &expired_entries {
                        if entry.tags.iter().any(|t| t == "web_artifact")
                            && let Some(semantic) = &_semantic
                            && let Err(e) = semantic.delete_web_artifact_points(&entry.staged_id).await
                        {
                            tracing::warn!(
                                staged_id = %entry.staged_id,
                                error = %e,
                                "Failed to cleanup vector points for expired web artifact"
                            );
                        }
                    }
                    for staged_id in expired_entries.iter().map(|e| &e.staged_id) {
                        memory.cache.invalidate(staged_id).await;
                    }
                    if !expired_entries.is_empty() {
                        tracing::info!(count = expired_entries.len(), "Expired staged entries removed from ephemeral memory");
                    }

                    if let Err(e) = memory.snapshot_to_disk(&vault_root).await {
                        tracing::error!("Daemon failed to snapshot memory: {}", e);
                    }
                }
                _ = promo_tick.tick() => {
                    if promotion_suppressed_during_step
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        tracing::trace!(
                            event = "promotion_tick_skipped_step_active",
                            "Skipping promotion/decay tick while orchestrator step is in progress"
                        );
                    } else {
                        evaluate_promotions_and_decay(&memory, &config).await;
                    }
                }
                _ = cancel_token.cancelled() => {
                    if let Err(e) = memory.snapshot_to_disk(&vault_root).await {
                        tracing::error!("Daemon failed to snapshot memory on cancellation: {}", e);
                    }
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_web_artifact_staging() {
        assert!(is_web_artifact_staging(
            &["web_artifact".into(), "external".into()],
            "anything"
        ));
        assert!(is_web_artifact_staging(
            &["news".into()],
            "web_artifact:uuid-here"
        ));
        assert!(!is_web_artifact_staging(
            &["user".into()],
            "hagbard_profile"
        ));
    }

    #[tokio::test]
    async fn test_ephemeral_insert_and_get() {
        let memory = EphemeralMemory::new("test_ws".to_string());

        let staged = memory
            .insert("key1", "value1", vec!["tag1".into()], 60)
            .await
            .unwrap();

        let result = memory.get("key1").await;
        assert_eq!(result, Some("value1".to_string()));

        let entry = memory.get_by_id(&staged.staged_id).await.unwrap();
        assert_eq!(entry.tags, vec!["tag1".to_string()]);
    }

    #[tokio::test]
    async fn test_upsert_by_title_replaces_prior() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        memory.insert("same", "a", vec![], 60).await.unwrap();
        memory
            .upsert_by_title("same", "b", vec![], 60)
            .await
            .unwrap();
        let v = memory.get("same").await;
        assert_eq!(v.as_deref(), Some("b"));
        let count = memory
            .list_entries()
            .into_iter()
            .filter(|e| e.title == "same")
            .count();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_ephemeral_absolute_ttl_enforcement() {
        let memory = EphemeralMemory::new("test_ws".to_string());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let past_timestamp = now.saturating_sub(1);

        // Manually insert into the cache to bypass the `insert` method's `now + ttl_secs` logic
        let expired_value = CacheValue {
            staged_id: "expired_id".to_string(),
            title: "expired_key".to_string(),
            data: "expired_data".to_string(),
            tags: vec![],
            expires_at: past_timestamp,
            node_id: uuid::Uuid::new_v4().to_string(),
            canonical_key: "expired_key".to_string(),
            tier: EphemeralTier::Session,
            promotion_score: 0.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: past_timestamp,
            last_seen_at: past_timestamp,
            kind: VaultKind::default(),
        };
        memory
            .cache
            .insert("expired_id".to_string(), expired_value)
            .await;

        let result = memory.get("expired_key").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_snapshot_and_load_preserves_valid_keys() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path();

        let memory = EphemeralMemory::new("test_ws".to_string());
        memory.insert("key1", "value1", vec![], 60).await.unwrap();

        memory.snapshot_to_disk(vault_root).await.unwrap();

        let loaded_memory = EphemeralMemory::load_from_disk("test_ws", vault_root, 10_000)
            .await
            .unwrap();

        let result = loaded_memory.get("key1").await;
        assert_eq!(result, Some("value1".to_string()));
    }

    #[tokio::test]
    async fn test_load_drops_stale_keys_from_disk() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path();

        // Setup initial cache state
        let memory = EphemeralMemory::new("test_ws".to_string());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let past_timestamp = now.saturating_sub(10);
        let future_timestamp = now + 60;

        let expired_value = CacheValue {
            staged_id: "expired_id".to_string(),
            title: "expired_key".to_string(),
            data: "expired".to_string(),
            tags: vec![],
            expires_at: past_timestamp,
            node_id: uuid::Uuid::new_v4().to_string(),
            canonical_key: "expired_key".to_string(),
            tier: EphemeralTier::Session,
            promotion_score: 0.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: past_timestamp,
            last_seen_at: past_timestamp,
            kind: VaultKind::default(),
        };
        let valid_value = CacheValue {
            staged_id: "valid_id".to_string(),
            title: "valid_key".to_string(),
            data: "valid".to_string(),
            tags: vec!["test".into()],
            expires_at: future_timestamp,
            node_id: uuid::Uuid::new_v4().to_string(),
            canonical_key: "valid_key".to_string(),
            tier: EphemeralTier::Session,
            promotion_score: 0.0,
            mention_count: 1,
            needs_review: false,
            first_seen_at: now,
            last_seen_at: now,
            kind: VaultKind::default(),
        };

        memory
            .cache
            .insert("expired_id".to_string(), expired_value)
            .await;
        memory
            .cache
            .insert("valid_id".to_string(), valid_value)
            .await;

        // Snapshot to disk
        memory.snapshot_to_disk(vault_root).await.unwrap();

        // Load
        let loaded_memory = EphemeralMemory::load_from_disk("test_ws", vault_root, 10_000)
            .await
            .unwrap();

        let r1 = loaded_memory.get("expired_key").await;
        let r2 = loaded_memory.get("valid_key").await;

        assert_eq!(r1, None);
        assert_eq!(r2, Some("valid".to_string()));
    }

    #[tokio::test]
    async fn test_daemon_snapshots_on_cancellation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path().to_path_buf();

        let memory = Arc::new(EphemeralMemory::new("daemon_test_ws".to_string()));
        memory.insert("key1", "value1", vec![], 60).await.unwrap();

        let cancel_token = CancellationToken::new();

        // Spawn the daemon with a very long interval
        spawn_snapshot_daemon(
            memory.clone(),
            vault_root.clone(),
            None,
            9999,
            cancel_token.clone(),
            Arc::new(crate::config::AppConfig::default()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );

        // Immediately cancel
        cancel_token.cancel();

        // Yield to let the daemon finish writing
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let path = crate::vault_layout::ephemeral_bin(&vault_root, "daemon_test_ws");
        assert!(path.exists(), "Snapshot file must exist after cancellation");

        let loaded = EphemeralMemory::load_from_disk("daemon_test_ws", &vault_root, 10_000)
            .await
            .unwrap();
        assert_eq!(loaded.get("key1").await, Some("value1".to_string()));
    }

    #[test]
    fn test_normalize_canonical_key_basic() {
        assert_eq!(
            normalize_canonical_key("Hagbard Profile"),
            "hagbard_profile"
        );
    }

    #[test]
    fn test_normalize_canonical_key_special_chars() {
        assert_eq!(
            normalize_canonical_key("API/REST endpoint"),
            "api_rest_endpoint"
        );
    }

    #[test]
    fn test_normalize_canonical_key_collapses_runs() {
        assert_eq!(normalize_canonical_key("  hello---world  "), "hello_world");
    }

    #[test]
    fn test_normalize_canonical_key_unicode() {
        // NFKC normalizes e.g. ﬁ -> fi
        assert_eq!(normalize_canonical_key("ﬁle_path"), "file_path");
    }

    #[tokio::test]
    async fn test_insert_populates_v2_fields() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let entry = memory
            .insert("My Title", "content", vec!["tag".into()], 60)
            .await
            .unwrap();

        assert!(!entry.node_id.is_empty());
        assert_eq!(entry.canonical_key, "my_title");
        assert_eq!(entry.tier, EphemeralTier::Session);
        assert_eq!(entry.mention_count, 1);
        assert_eq!(entry.promotion_score, 0.0);
        assert!(!entry.needs_review);
        assert!(entry.first_seen_at > 0);
        assert_eq!(entry.first_seen_at, entry.last_seen_at);
        assert_eq!(entry.kind, VaultKind::Synthesis);
    }

    #[tokio::test]
    async fn test_insert_with_tier_uses_given_tier() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let entry = memory
            .insert_with_tier(
                "t",
                "c",
                vec![],
                60,
                EphemeralTier::Promote,
                VaultKind::Discourse,
            )
            .await
            .unwrap();
        assert_eq!(entry.tier, EphemeralTier::Promote);
        assert_eq!(entry.kind, VaultKind::Discourse);
    }

    #[tokio::test]
    async fn test_upsert_preserves_node_id() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let first = memory
            .insert("same_title", "v1", vec!["a".into()], 60)
            .await
            .unwrap();
        let second = memory
            .upsert_by_title("same_title", "v2", vec!["b".into()], 60)
            .await
            .unwrap();

        assert_eq!(
            first.node_id, second.node_id,
            "node_id must be preserved across upserts"
        );
        assert_eq!(second.mention_count, 2);
        assert_eq!(second.first_seen_at, first.first_seen_at);
    }

    #[tokio::test]
    async fn test_get_by_canonical_key() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        memory
            .insert(
                "Coffee Preference",
                "black, no sugar",
                vec!["user".into()],
                60,
            )
            .await
            .unwrap();

        let found = memory.get_by_canonical_key("coffee_preference").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().data, "black, no sugar");
    }

    #[tokio::test]
    async fn test_get_by_node_id() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let entry = memory
            .insert("lookup", "by node", vec![], 60)
            .await
            .unwrap();

        let found = memory.get_by_node_id(&entry.node_id).await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().staged_id, entry.staged_id);
    }

    #[tokio::test]
    async fn test_promotion_engine_promotes_on_threshold() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let mut config = crate::config::AppConfig::default();
        config.promotion_threshold_session_to_scratch = 3.0;
        config.promotion_decay_per_tick = 0.0; // disable decay for this test

        // Insert with score above threshold
        let mut entry = memory
            .insert("promotable", "content", vec![], 300)
            .await
            .unwrap();
        entry.promotion_score = 5.0;
        entry.tier = EphemeralTier::Session;
        memory
            .cache
            .insert(entry.staged_id.clone(), entry.clone())
            .await;

        evaluate_promotions_and_decay(&memory, &config).await;

        let updated = memory.get_by_id(&entry.staged_id).await.unwrap();
        assert_eq!(
            updated.tier,
            EphemeralTier::Scratch,
            "should have been promoted to scratch"
        );
    }

    #[tokio::test]
    async fn test_promotion_engine_applies_decay() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let mut config = crate::config::AppConfig::default();
        config.promotion_decay_per_tick = 1.0;
        config.promotion_threshold_session_to_scratch = 100.0; // won't promote

        let mut entry = memory
            .insert("decaying", "content", vec![], 300)
            .await
            .unwrap();
        entry.promotion_score = 3.0;
        memory
            .cache
            .insert(entry.staged_id.clone(), entry.clone())
            .await;

        evaluate_promotions_and_decay(&memory, &config).await;

        let updated = memory.get_by_id(&entry.staged_id).await.unwrap();
        assert!(
            (updated.promotion_score - 2.0).abs() < 0.01,
            "score should have decayed by 1.0"
        );
    }

    #[tokio::test]
    async fn test_promotion_engine_demotes_on_low_score() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let mut config = crate::config::AppConfig::default();
        config.promotion_threshold_session_to_scratch = 3.0;
        config.promotion_decay_per_tick = 0.0;

        // Start at scratch with very low score (below 50% of session->scratch threshold)
        let mut entry = memory
            .insert("demotable", "content", vec![], 300)
            .await
            .unwrap();
        entry.tier = EphemeralTier::Scratch;
        entry.promotion_score = 0.5; // below 3.0 * 0.5 = 1.5
        memory
            .cache
            .insert(entry.staged_id.clone(), entry.clone())
            .await;

        evaluate_promotions_and_decay(&memory, &config).await;

        let updated = memory.get_by_id(&entry.staged_id).await.unwrap();
        assert_eq!(
            updated.tier,
            EphemeralTier::Session,
            "should have been demoted back to session"
        );
    }

    #[tokio::test]
    async fn test_needs_review_blocks_promotion() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        let mut config = crate::config::AppConfig::default();
        config.promotion_threshold_session_to_scratch = 1.0;
        config.promotion_decay_per_tick = 0.0;

        let mut entry = memory
            .insert("contested", "content", vec![], 300)
            .await
            .unwrap();
        entry.promotion_score = 10.0;
        entry.needs_review = true;
        memory
            .cache
            .insert(entry.staged_id.clone(), entry.clone())
            .await;

        evaluate_promotions_and_decay(&memory, &config).await;

        let updated = memory.get_by_id(&entry.staged_id).await.unwrap();
        assert_eq!(
            updated.tier,
            EphemeralTier::Session,
            "needs_review should block promotion"
        );
    }
}
