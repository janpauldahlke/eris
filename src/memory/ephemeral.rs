use serde::{Deserialize, Serialize};
use moka::future::Cache;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::executive::error::Result;

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheValue {
    pub data: String,
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

    pub async fn insert(&self, key: &str, value: &str, ttl_secs: u64) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards");
        let expires_at = now.as_secs() + ttl_secs;
        
        let cache_value = CacheValue {
            data: value.to_string(),
            expires_at,
        };
        
        self.cache.insert(key.to_string(), cache_value).await;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> Option<String> {
        let val = self.cache.get(key).await?;
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
            
        if now >= val.expires_at {
            self.cache.invalidate(key).await;
            None
        } else {
            Some(val.data)
        }
    }

    pub async fn snapshot_to_disk(&self, vault_root: &std::path::Path) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs();
        
        let mut entries = Vec::new();
        for (k, v) in self.cache.iter() {
            if v.expires_at > now {
                entries.push((k.to_string(), v.clone()));
            }
        }
        
        let path = vault_root.join(format!(".fcp/ephemeral_{}.bin", self.workspace));
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
            
        let path = vault_root.join(format!(".fcp/ephemeral_{}.bin", workspace));
        
        if path.exists() {
            let data = tokio::fs::read(path).await.map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                workspace: workspace.to_string(),
                reason: e.to_string(),
            })?;
            if !data.is_empty() {
                let ws = workspace.to_string();
                let entries: Vec<(String, CacheValue)> = tokio::task::spawn_blocking(move || bincode::deserialize(&data))
                    .await
                    .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                        workspace: ws.clone(),
                        reason: e.to_string(),
                    })?
                    .map_err(|e| crate::executive::error::FcpError::WorkspaceFault {
                        workspace: ws,
                        reason: e.to_string(),
                    })?;
                
                let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs();
                for (k, v) in entries {
                    if v.expires_at > now {
                        cache.insert(k, v).await;
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

pub fn spawn_snapshot_daemon(
    memory: Arc<EphemeralMemory>,
    vault_root: std::path::PathBuf,
    interval_secs: u64,
    cancel_token: CancellationToken
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        // Consume the first tick, which happens immediately
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
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

    #[tokio::test]
    async fn test_ephemeral_insert_and_get() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        
        memory.insert("key1", "value1", 60).await.unwrap();
        
        let result = memory.get("key1").await;
        assert_eq!(result, Some("value1".to_string()));
    }

    #[tokio::test]
    async fn test_ephemeral_absolute_ttl_enforcement() {
        let memory = EphemeralMemory::new("test_ws".to_string());
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let past_timestamp = now.saturating_sub(1);
        
        // Manually insert into the cache to bypass the `insert` method's `now + ttl_secs` logic
        let expired_value = CacheValue {
            data: "expired_data".to_string(),
            expires_at: past_timestamp,
        };
        memory.cache.insert("expired_key".to_string(), expired_value).await;
        
        let result = memory.get("expired_key").await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_snapshot_and_load_preserves_valid_keys() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path();
        
        let memory = EphemeralMemory::new("test_ws".to_string());
        memory.insert("key1", "value1", 60).await.unwrap();
        
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
            data: "expired".to_string(),
            expires_at: past_timestamp,
        };
        let valid_value = CacheValue {
            data: "valid".to_string(),
            expires_at: future_timestamp,
        };
        
        memory.cache.insert("expired_key".to_string(), expired_value).await;
        memory.cache.insert("valid_key".to_string(), valid_value).await;
        
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
        memory.insert("key1", "value1", 60).await.unwrap();
        
        let cancel_token = CancellationToken::new();
        
        // Spawn the daemon with a very long interval
        spawn_snapshot_daemon(
            memory.clone(),
            vault_root.clone(),
            9999,
            cancel_token.clone(),
        );
        
        // Immediately cancel
        cancel_token.cancel();
        
        // Yield to let the daemon finish writing
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        let path = vault_root.join(".fcp/ephemeral_daemon_test_ws.bin");
        assert!(path.exists(), "Snapshot file must exist after cancellation");
        
        let loaded = EphemeralMemory::load_from_disk("daemon_test_ws", &vault_root, 10_000).await.unwrap();
        assert_eq!(loaded.get("key1").await, Some("value1".to_string()));
    }
}
