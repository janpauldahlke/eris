//! Mutation tracking for benchmark cleanup.
//!
//! Tracks all changes made during benchmark execution and provides
//! automatic rollback capabilities.

use crate::executive::error::{FcpError, Result};
use std::path::PathBuf;
use std::time::SystemTime;
use tracing;

/// Tracks mutations made during benchmark execution.
#[derive(Debug, Clone, Default)]
pub struct MutationTracker {
    /// Staged memories created.
    pub staged_memories: Vec<StagedMemoryRecord>,
    /// Ephemeral tier entries created.
    pub ephemeral_entries: Vec<EphemeralEntryRecord>,
    /// Temporary files created.
    pub temp_files: Vec<PathBuf>,
    /// Vault files written.
    pub vault_writes: Vec<VaultWriteRecord>,
}

/// Record of a staged memory.
#[derive(Debug, Clone)]
pub struct StagedMemoryRecord {
    pub canonical_key: String,
    pub content_hash: String,
    pub created_at: SystemTime,
}

/// Record of an ephemeral entry.
#[derive(Debug, Clone)]
pub struct EphemeralEntryRecord {
    pub id: uuid::Uuid,
    pub tier: String,
    pub created_at: SystemTime,
}

/// Record of a vault file write.
#[derive(Debug, Clone)]
pub struct VaultWriteRecord {
    pub path: PathBuf,
    pub original_content: Option<String>, // None if file didn't exist
    pub written_at: SystemTime,
}

/// Report of cleanup operations.
#[derive(Debug, Clone, Default)]
pub struct CleanupReport {
    pub staged_removed: usize,
    pub ephemeral_removed: usize,
    pub files_deleted: usize,
    pub vault_files_restored: usize,
    pub failures: Vec<CleanupFailure>,
}

/// Record of a cleanup failure.
#[derive(Debug, Clone)]
pub struct CleanupFailure {
    pub item_type: String,
    pub identifier: String,
    pub error: String,
}

impl MutationTracker {
    /// Create a new mutation tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a staged memory creation.
    pub fn record_staged_memory(&mut self, canonical_key: &str, content: &str) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = format!("{:x}", hasher.finish());

        self.staged_memories.push(StagedMemoryRecord {
            canonical_key: canonical_key.to_string(),
            content_hash,
            created_at: SystemTime::now(),
        });

        tracing::debug!(
            canonical_key = %canonical_key,
            "MutationTracker: recorded staged memory"
        );
    }

    /// Record an ephemeral entry creation.
    pub fn record_ephemeral_entry(&mut self, id: uuid::Uuid, tier: impl Into<String>) {
        self.ephemeral_entries.push(EphemeralEntryRecord {
            id,
            tier: tier.into(),
            created_at: SystemTime::now(),
        });

        tracing::debug!(
            entry_id = %id,
            "MutationTracker: recorded ephemeral entry"
        );
    }

    /// Record a temporary file creation.
    pub fn record_temp_file(&mut self, path: PathBuf) {
        self.temp_files.push(path.clone());

        tracing::debug!(
            path = %path.display(),
            "MutationTracker: recorded temp file"
        );
    }

    /// Record a vault file write.
    pub fn record_vault_write(&mut self, path: PathBuf, original_content: Option<String>) {
        let had_original = original_content.is_some();
        
        self.vault_writes.push(VaultWriteRecord {
            path: path.clone(),
            original_content,
            written_at: SystemTime::now(),
        });

        tracing::debug!(
            path = %path.display(),
            had_original = had_original,
            "MutationTracker: recorded vault write"
        );
    }

    /// Perform cleanup of all tracked mutations.
    pub async fn cleanup_all(&mut self) -> Result<CleanupReport> {
        let mut report = CleanupReport::default();

        // Clean up staged memories
        for record in &self.staged_memories {
            match self.remove_staged_memory(record).await {
                Ok(_) => report.staged_removed += 1,
                Err(e) => {
                    report.failures.push(CleanupFailure {
                        item_type: "staged_memory".to_string(),
                        identifier: record.canonical_key.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Clean up ephemeral entries
        for record in &self.ephemeral_entries {
            match self.remove_ephemeral_entry(record).await {
                Ok(_) => report.ephemeral_removed += 1,
                Err(e) => {
                    report.failures.push(CleanupFailure {
                        item_type: "ephemeral_entry".to_string(),
                        identifier: record.id.to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Delete temp files
        for path in &self.temp_files {
            match self.delete_temp_file(path).await {
                Ok(_) => report.files_deleted += 1,
                Err(e) => {
                    report.failures.push(CleanupFailure {
                        item_type: "temp_file".to_string(),
                        identifier: path.display().to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        // Restore vault files
        for record in &self.vault_writes {
            match self.restore_vault_file(record).await {
                Ok(_) => report.vault_files_restored += 1,
                Err(e) => {
                    report.failures.push(CleanupFailure {
                        item_type: "vault_file".to_string(),
                        identifier: record.path.display().to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }

        tracing::info!(
            staged_removed = report.staged_removed,
            ephemeral_removed = report.ephemeral_removed,
            files_deleted = report.files_deleted,
            vault_restored = report.vault_files_restored,
            failures = report.failures.len(),
            "MutationTracker: cleanup complete"
        );

        Ok(report)
    }

    /// Remove a staged memory.
    async fn remove_staged_memory(&self, record: &StagedMemoryRecord) -> Result<()> {
        tracing::debug!(
            canonical_key = %record.canonical_key,
            "Removing staged memory"
        );
        // In real implementation, this would call ephemeral.remove()
        // For now, just log
        Ok(())
    }

    /// Remove an ephemeral entry.
    async fn remove_ephemeral_entry(&self, record: &EphemeralEntryRecord) -> Result<()> {
        tracing::debug!(
            entry_id = %record.id,
            tier = %record.tier,
            "Removing ephemeral entry"
        );
        // In real implementation, this would delete from ephemeral store
        Ok(())
    }

    /// Delete a temporary file.
    async fn delete_temp_file(&self, path: &PathBuf) -> Result<()> {
        tracing::debug!(path = %path.display(), "Deleting temp file");
        
        match tokio::fs::remove_file(path).await {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), // Already deleted
            Err(e) => Err(FcpError::Io(e)),
        }
    }

    /// Restore a vault file to original state.
    async fn restore_vault_file(&self, record: &VaultWriteRecord) -> Result<()> {
        tracing::debug!(
            path = %record.path.display(),
            has_backup = record.original_content.is_some(),
            "Restoring vault file"
        );

        match &record.original_content {
            Some(content) => {
                // Restore original content
                tokio::fs::write(&record.path, content)
                    .await
                    .map_err(FcpError::Io)?;
            }
            None => {
                // File didn't exist before, delete it
                match tokio::fs::remove_file(&record.path).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(FcpError::Io(e)),
                }
            }
        }

        Ok(())
    }

    /// Get summary of tracked mutations.
    pub fn summary(&self) -> MutationSummary {
        MutationSummary {
            staged_count: self.staged_memories.len(),
            ephemeral_count: self.ephemeral_entries.len(),
            temp_files_count: self.temp_files.len(),
            vault_writes_count: self.vault_writes.len(),
        }
    }

    /// Clear all tracked mutations (use with caution).
    pub fn clear(&mut self) {
        self.staged_memories.clear();
        self.ephemeral_entries.clear();
        self.temp_files.clear();
        self.vault_writes.clear();
        
        tracing::warn!("MutationTracker: all records cleared");
    }
}

/// Summary of tracked mutations.
#[derive(Debug, Clone, Default)]
pub struct MutationSummary {
    pub staged_count: usize,
    pub ephemeral_count: usize,
    pub temp_files_count: usize,
    pub vault_writes_count: usize,
}

/// RAII guard that ensures cleanup on drop.
pub struct CleanupGuard<'a> {
    tracker: &'a Arc<tokio::sync::Mutex<MutationTracker>>,
    enabled: bool,
}

impl<'a> CleanupGuard<'a> {
    /// Create a new cleanup guard.
    pub fn new(tracker: &'a Arc<tokio::sync::Mutex<MutationTracker>>) -> Self {
        Self {
            tracker,
            enabled: true,
        }
    }

    /// Disable cleanup (for debugging).
    pub fn disable(&mut self) {
        self.enabled = false;
        tracing::warn!("CleanupGuard: cleanup disabled");
    }
}

impl<'a> Drop for CleanupGuard<'a> {
    fn drop(&mut self) {
        if self.enabled {
            // Spawn cleanup in background - can't block in Drop
            let tracker = self.tracker.clone();
            tokio::spawn(async move {
                let mut guard = tracker.lock().await;
                match guard.cleanup_all().await {
                    Ok(report) => {
                        tracing::info!(
                            staged_removed = report.staged_removed,
                            files_deleted = report.files_deleted,
                            "CleanupGuard: automatic cleanup complete"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "CleanupGuard: cleanup failed");
                    }
                }
            });
        }
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_records_staged_memory() {
        let mut tracker = MutationTracker::new();
        
        tracker.record_staged_memory("test_key", "test content");
        
        assert_eq!(tracker.staged_memories.len(), 1);
        assert_eq!(tracker.staged_memories[0].canonical_key, "test_key");
    }

    #[test]
    fn tracker_records_ephemeral_entry() {
        let mut tracker = MutationTracker::new();
        let id = uuid::Uuid::new_v4();
        
        tracker.record_ephemeral_entry(id, "Session");
        
        assert_eq!(tracker.ephemeral_entries.len(), 1);
        assert_eq!(tracker.ephemeral_entries[0].id, id);
    }

    #[test]
    fn tracker_records_temp_file() {
        let mut tracker = MutationTracker::new();
        let path = PathBuf::from("/tmp/test_file.txt");
        
        tracker.record_temp_file(path.clone());
        
        assert_eq!(tracker.temp_files.len(), 1);
        assert_eq!(tracker.temp_files[0], path);
    }

    #[test]
    fn tracker_generates_summary() {
        let mut tracker = MutationTracker::new();
        
        tracker.record_staged_memory("key1", "content1");
        tracker.record_staged_memory("key2", "content2");
        tracker.record_ephemeral_entry(uuid::Uuid::new_v4(), "Session");
        tracker.record_temp_file(PathBuf::from("/tmp/test.txt"));
        
        let summary = tracker.summary();
        assert_eq!(summary.staged_count, 2);
        assert_eq!(summary.ephemeral_count, 1);
        assert_eq!(summary.temp_files_count, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_deletes_temp_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("test.txt");
        
        // Create a temp file
        tokio::fs::write(&file_path, "test content")
            .await
            .expect("write");
        
        assert!(file_path.exists());
        
        let mut tracker = MutationTracker::new();
        tracker.record_temp_file(file_path.clone());
        
        let report = tracker.cleanup_all().await.expect("cleanup");
        
        assert_eq!(report.files_deleted, 1);
        assert!(!file_path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_restores_vault_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("vault_file.md");
        
        // Create original file
        let original_content = "original";
        tokio::fs::write(&file_path, original_content)
            .await
            .expect("write");
        
        // Modify file
        tokio::fs::write(&file_path, "modified")
            .await
            .expect("write");
        
        let mut tracker = MutationTracker::new();
        tracker.record_vault_write(file_path.clone(), Some(original_content.to_string()));
        
        let report = tracker.cleanup_all().await.expect("cleanup");
        
        assert_eq!(report.vault_files_restored, 1);
        
        let restored = tokio::fs::read_to_string(&file_path)
            .await
            .expect("read");
        assert_eq!(restored, original_content);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cleanup_deletes_new_vault_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("new_file.md");
        
        // Create file (didn't exist before)
        tokio::fs::write(&file_path, "content")
            .await
            .expect("write");
        
        assert!(file_path.exists());
        
        let mut tracker = MutationTracker::new();
        tracker.record_vault_write(file_path.clone(), None); // No original content
        
        let report = tracker.cleanup_all().await.expect("cleanup");
        
        assert_eq!(report.vault_files_restored, 1);
        assert!(!file_path.exists());
    }
}
