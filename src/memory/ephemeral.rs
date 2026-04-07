use serde::{Deserialize, Serialize};
use moka::future::Cache;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::executive::error::Result;

use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use crate::config::MemoryRoutingConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheValue {
    pub staged_id: String,
    pub title: String,
    pub data: String,
    pub tags: Vec<String>,
    pub expires_at: u64, // Absolute UNIX timestamp
}

pub struct EphemeralMemory {
    pub cache: Cache<String, CacheValue>,
    pub workspace: String,
}

impl EphemeralMemory {
    pub fn new(workspace: String) -> Self {
        let cache = Cache::builder()
            .max_capacity(10_000)
            .build();
            
        Self { cache, workspace }
    }

    pub async fn insert(&self, title: &str, value: &str, tags: Vec<String>, ttl_secs: u64) -> Result<CacheValue> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let expires_at = now.as_secs() + ttl_secs;
        let staged_id = uuid::Uuid::new_v4().to_string();

        let cache_value = CacheValue {
            staged_id: staged_id.clone(),
            title: title.to_string(),
            data: value.to_string(),
            tags,
            expires_at,
        };

        self.cache.insert(staged_id, cache_value.clone()).await;
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
    pub async fn upsert_by_title(
        &self,
        title: &str,
        value: &str,
        tags: Vec<String>,
        ttl_secs: u64,
    ) -> Result<CacheValue> {
        self.invalidate_by_title(title).await;
        self.insert(title, value, tags, ttl_secs).await
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
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        
        let entries: Vec<CacheValue> = self
            .cache
            .iter()
            .filter_map(|(_, v)| (v.expires_at > now).then_some(v.clone()))
            .collect();
        
        let path = crate::vault_layout::ephemeral_bin(vault_root, &self.workspace);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                workspace: self.workspace.clone(),
                reason: e.to_string(),
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
            
        tokio::fs::write(path, serialized).await.map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
            workspace: self.workspace.clone(),
            reason: e.to_string(),
        })?;
        
        Ok(())
    }

    pub async fn load_from_disk(workspace: &str, vault_root: &std::path::Path, max_capacity: u64) -> Result<Self> {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .build();
            
        let path = crate::vault_layout::ephemeral_bin(vault_root, workspace);
        
        if path.exists() {
            let data = tokio::fs::read(path).await.map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                workspace: workspace.to_string(),
                reason: e.to_string(),
            })?;
            if !data.is_empty() {
                let ws = workspace.to_string();
                let entries: Vec<CacheValue> = tokio::task::spawn_blocking(move || bincode::deserialize(&data))
                    .await
                    .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                        workspace: ws.clone(),
                        reason: e.to_string(),
                    })?
                    .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                        workspace: ws,
                        reason: e.to_string(),
                    })?;
                
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
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

/// Staged rows from `web:fetch` must not be promoted to vault markdown (bloated HTML/JSON).
/// They are indexed in Qdrant at fetch time; `memory:commit` treats them as semantic-only.
pub fn is_web_artifact_staging(tags: &[String], title: &str) -> bool {
    tags.iter().any(|t| t == "web_artifact") || title.starts_with("web_artifact:")
}

pub fn resolve_vault_subdir<'a>(tags: &[String], routing: &'a MemoryRoutingConfig) -> &'a str {
    fn split_fragments(tag: &str) -> Vec<String> {
        tag.to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    let normalized_keywords: Vec<(&str, Vec<String>)> = routing
        .rules
        .iter()
        .map(|rule| {
            let keys = rule
                .keywords
                .iter()
                .flat_map(|k| split_fragments(k))
                .collect::<Vec<_>>();
            (rule.folder.as_str(), keys)
        })
        .collect();

    let mut normalized = Vec::new();
    for tag in tags {
        normalized.extend(split_fragments(tag));
    }

    for token in &normalized {
        for (folder, keywords) in &normalized_keywords {
            if keywords.iter().any(|k| k == token) {
                return folder;
            }
        }
    }
    routing.default.as_str()
}

pub fn spawn_snapshot_daemon(
    memory: Arc<EphemeralMemory>,
    vault_root: PathBuf,
    _semantic: Option<Arc<crate::memory::semantic::SemanticBrain>>,
    interval_secs: u64,
    cancel_token: CancellationToken,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
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
        assert!(is_web_artifact_staging(&["web_artifact".into(), "external".into()], "anything"));
        assert!(is_web_artifact_staging(&["news".into()], "web_artifact:uuid-here"));
        assert!(!is_web_artifact_staging(&["user".into()], "hagbard_profile"));
    }

    #[test]
    fn test_resolve_vault_subdir_splits_compound_tags() {
        let tags = vec!["user/preference".to_string()];
        let routing = MemoryRoutingConfig::default();
        assert_eq!(resolve_vault_subdir(&tags, &routing), "40_User");
    }

    #[test]
    fn test_resolve_vault_subdir_routes_technical_knowledge_to_semantic() {
        let tags = vec!["system-knowledge".to_string(), "programmer".to_string()];
        let routing = MemoryRoutingConfig::default();
        assert_eq!(resolve_vault_subdir(&tags, &routing), "20_Semantic");
    }

    #[tokio::test]
    async fn test_ephemeral_insert_and_get() {
        let memory = EphemeralMemory::new("test_ws".to_string());

        let staged = memory.insert("key1", "value1", vec!["tag1".into()], 60).await.unwrap();

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
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let past_timestamp = now.saturating_sub(1);
        
        // Manually insert into the cache to bypass the `insert` method's `now + ttl_secs` logic
        let expired_value = CacheValue {
            staged_id: "expired_id".to_string(),
            title: "expired_key".to_string(),
            data: "expired_data".to_string(),
            tags: vec![],
            expires_at: past_timestamp,
        };
        memory.cache.insert("expired_id".to_string(), expired_value).await;
        
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
        
        let loaded_memory = EphemeralMemory::load_from_disk("test_ws", vault_root, 10_000).await.unwrap();
        
        let result = loaded_memory.get("key1").await;
        assert_eq!(result, Some("value1".to_string()));
    }

    #[tokio::test]
    async fn test_load_drops_stale_keys_from_disk() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path();
        
        // Setup initial cache state
        let memory = EphemeralMemory::new("test_ws".to_string());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let past_timestamp = now.saturating_sub(10);
        let future_timestamp = now + 60;
        
        let expired_value = CacheValue {
            staged_id: "expired_id".to_string(),
            title: "expired_key".to_string(),
            data: "expired".to_string(),
            tags: vec![],
            expires_at: past_timestamp,
        };
        let valid_value = CacheValue {
            staged_id: "valid_id".to_string(),
            title: "valid_key".to_string(),
            data: "valid".to_string(),
            tags: vec!["test".into()],
            expires_at: future_timestamp,
        };
        
        memory.cache.insert("expired_id".to_string(), expired_value).await;
        memory.cache.insert("valid_id".to_string(), valid_value).await;
        
        // Snapshot to disk
        memory.snapshot_to_disk(vault_root).await.unwrap();
        
        // Load
        let loaded_memory = EphemeralMemory::load_from_disk("test_ws", vault_root, 10_000).await.unwrap();
        
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
        );
        
        // Immediately cancel
        cancel_token.cancel();
        
        // Yield to let the daemon finish writing
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        let path = crate::vault_layout::ephemeral_bin(&vault_root, "daemon_test_ws");
        assert!(path.exists(), "Snapshot file must exist after cancellation");
        
        let loaded = EphemeralMemory::load_from_disk("daemon_test_ws", &vault_root, 10_000).await.unwrap();
        assert_eq!(loaded.get("key1").await, Some("value1".to_string()));
    }
}
