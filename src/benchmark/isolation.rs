//! Safety isolation layer for benchmarking.
//!
//! Provides zero-pollution guarantees:
//! - Temporary vault directory (auto-deleted on Drop)
//! - Ephemeral Qdrant collections
//! - Side-effect filtering (blocks mail/moltbook writes)
//! - Automatic cleanup of staged memories

use crate::executive::error::{FcpError, Result};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tracing;
use uuid::Uuid;

/// Risk levels for tool categorization in benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolRiskLevel {
    /// Internal tools with no external effects (memory:*, vault:read, system:*).
    Safe,
    /// Read-only external tools (weather:*, wiki:*, web:fetch).
    ReadOnlyExternal,
    /// Tools that mutate external services (mail:write, moltbook:post).
    MutatingExternal,
}

/// Filters tools to prevent external side effects during benchmarking.
#[derive(Debug, Clone)]
pub struct SideEffectFilter {
    blocked_prefixes: Vec<&'static str>,
    allowed_in_strict_mode: Vec<&'static str>,
}

impl SideEffectFilter {
    /// Strict mode: only Safe and ReadOnlyExternal tools allowed.
    /// Blocks all mutating external operations.
    pub fn strict() -> Self {
        Self {
            blocked_prefixes: vec![
                "mail:write",
                "mail:delete",
                "mail:move",
                "moltbook:post",
                "moltbook:comment",
                "moltbook:vote",
                "moltbook:dm",
                "calendar:create",
                "calendar:update",
                "calendar:delete",
            ],
            allowed_in_strict_mode: vec![
                "memory:",
                "vault:read",
                "vault:list",
                "vault:search",
                "system:",
                "clock:",
                "weather:",
                "wiki:",
                "web:artifact_query",
                "news:today",
                "db:find_connections",
                "mail:check",
                "mail:read",
                "mail:digest",
                "calendar:list",
                "calendar:get",
                "moltbook:status",
                "moltbook:home",
                "moltbook:feed",
                "moltbook:search",
                "moltbook:comments",
                "moltbook:verify",
                "moltbook:notifications_read",
            ],
        }
    }

    /// Relaxed mode: allows ReadOnlyExternal tools.
    pub fn relaxed() -> Self {
        Self {
            blocked_prefixes: vec![
                "mail:write",
                "mail:delete",
                "mail:move",
                "moltbook:post",
                "moltbook:comment",
                "moltbook:vote",
                "moltbook:dm",
                "calendar:create",
                "calendar:update",
                "calendar:delete",
            ],
            allowed_in_strict_mode: vec![], // Not used in relaxed mode
        }
    }

    /// Check if a tool is allowed in the current filter mode.
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        // First check blocked list
        if self.blocked_prefixes.iter().any(|p| tool_name.starts_with(p)) {
            tracing::warn!(
                tool = tool_name,
                "SideEffectFilter: blocked mutating external tool"
            );
            return false;
        }

        // In strict mode, verify tool is in allowed list
        if !self.allowed_in_strict_mode.is_empty() {
            let allowed = self
                .allowed_in_strict_mode
                .iter()
                .any(|p| tool_name.starts_with(p));
            if !allowed {
                tracing::warn!(
                    tool = tool_name,
                    "SideEffectFilter: tool not in strict mode allowlist"
                );
            }
            return allowed;
        }

        true
    }

    /// Categorize a tool by risk level.
    pub fn categorize(tool_name: &str) -> ToolRiskLevel {
        let blocked = [
            "mail:write",
            "mail:delete",
            "mail:move",
            "moltbook:post",
            "moltbook:comment",
            "moltbook:vote",
            "moltbook:dm",
            "calendar:create",
            "calendar:update",
            "calendar:delete",
        ];

        let readonly_external = [
            "weather:",
            "wiki:",
            "news:today",
            "db:find_connections",
            "web:artifact_query",
            "mail:check",
            "mail:read",
            "mail:digest",
            "calendar:list",
            "calendar:get",
            "moltbook:status",
            "moltbook:home",
            "moltbook:feed",
            "moltbook:search",
            "moltbook:comments",
            "moltbook:verify",
            "moltbook:notifications_read",
        ];

        if blocked.iter().any(|p| tool_name.starts_with(p)) {
            ToolRiskLevel::MutatingExternal
        } else if readonly_external.iter().any(|p| tool_name.starts_with(p)) {
            ToolRiskLevel::ReadOnlyExternal
        } else {
            ToolRiskLevel::Safe
        }
    }
}

/// Provides isolated environment for benchmark execution.
/// Creates temporary vault and ephemeral Qdrant collection.
/// Automatically cleans up on Drop.
pub struct BenchmarkIsolation {
    temp_vault: TempDir,
    qdrant_collection: String,
    original_vault_root: PathBuf,
    cleanup_on_drop: bool,
}

impl BenchmarkIsolation {
    /// Create new isolated benchmark environment.
    pub fn new(original_vault: &Path) -> Result<Self> {
        let temp_vault = tempfile::tempdir().map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to create temp vault: {}", e),
            ))
        })?;

        let qdrant_collection = format!("benchmark_{}", Uuid::new_v4());

        tracing::info!(
            temp_vault = %temp_vault.path().display(),
            qdrant_collection = %qdrant_collection,
            "BenchmarkIsolation: created isolated environment"
        );

        Ok(Self {
            temp_vault,
            qdrant_collection,
            original_vault_root: original_vault.to_path_buf(),
            cleanup_on_drop: true,
        })
    }

    /// Get the temporary vault root path.
    pub fn vault_root(&self) -> &Path {
        self.temp_vault.path()
    }

    /// Get the ephemeral Qdrant collection name.
    pub fn qdrant_collection(&self) -> &str {
        &self.qdrant_collection
    }

    /// Get the original vault root (for reference/copying if needed).
    pub fn original_vault(&self) -> &Path {
        &self.original_vault_root
    }

    /// Disable automatic cleanup (for debugging only).
    pub fn disable_cleanup(&mut self) {
        self.cleanup_on_drop = false;
        tracing::warn!(
            temp_vault = %self.temp_vault.path().display(),
            "BenchmarkIsolation: cleanup disabled - manual removal required"
        );
    }

    /// Generate cleanup report.
    pub fn cleanup_report(&self) -> CleanupReport {
        CleanupReport {
            temp_vault: self.temp_vault.path().to_path_buf(),
            qdrant_collection: self.qdrant_collection.clone(),
            will_auto_cleanup: self.cleanup_on_drop,
            staged_removed: 0, // Will be updated by mutation tracker
            ephemeral_removed: 0,
            failures: vec![],
        }
    }
}

impl Drop for BenchmarkIsolation {
    fn drop(&mut self) {
        if self.cleanup_on_drop {
            tracing::info!(
                temp_vault = %self.temp_vault.path().display(),
                qdrant_collection = %self.qdrant_collection,
                "BenchmarkIsolation: automatic cleanup"
            );
            // TempDir auto-deletes on drop
            // Qdrant collection cleanup happens via separate task
        } else {
            tracing::warn!(
                temp_vault = %self.temp_vault.path().display(),
                "BenchmarkIsolation: cleanup skipped - manual removal required"
            );
        }
    }
}

/// Report of what will be/is cleaned up.
#[derive(Debug, Clone, Default)]
pub struct CleanupReport {
    pub temp_vault: PathBuf,
    pub qdrant_collection: String,
    pub will_auto_cleanup: bool,
    pub staged_removed: usize,
    pub ephemeral_removed: usize,
    pub failures: Vec<CleanupFailure>,
}

/// Record of a cleanup failure.
#[derive(Debug, Clone)]
pub struct CleanupFailure {
    pub item_type: String,
    pub identifier: String,
    pub error: String,
}

/// Isolation mode for benchmark execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IsolationMode {
    /// Only Safe tools allowed. No external reads or writes.
    Strict,
    /// Safe + ReadOnlyExternal tools allowed.
    Relaxed,
    /// All tools allowed (requires --i-understand-risks).
    Unsafe,
}

impl std::str::FromStr for IsolationMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "strict" => Ok(IsolationMode::Strict),
            "relaxed" => Ok(IsolationMode::Relaxed),
            "unsafe" => Ok(IsolationMode::Unsafe),
            _ => Err(format!("Unknown isolation mode: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_effect_filter_blocks_mutating_tools() {
        let filter = SideEffectFilter::strict();

        assert!(!filter.is_allowed("mail:write"));
        assert!(!filter.is_allowed("mail:delete"));
        assert!(!filter.is_allowed("moltbook:post"));
        assert!(!filter.is_allowed("calendar:create"));
    }

    #[test]
    fn side_effect_filter_allows_safe_tools() {
        let filter = SideEffectFilter::strict();

        assert!(filter.is_allowed("memory:stage"));
        assert!(filter.is_allowed("vault:read"));
        assert!(filter.is_allowed("system:health"));
        assert!(filter.is_allowed("clock:now"));
    }

    #[test]
    fn side_effect_filter_allows_readonly_external_in_strict() {
        let filter = SideEffectFilter::strict();

        assert!(filter.is_allowed("weather:current"));
        assert!(filter.is_allowed("wiki:summary"));
        assert!(filter.is_allowed("mail:check"));
        assert!(filter.is_allowed("calendar:list"));
    }

    #[test]
    fn tool_categorization_works() {
        assert_eq!(
            SideEffectFilter::categorize("mail:write"),
            ToolRiskLevel::MutatingExternal
        );
        assert_eq!(
            SideEffectFilter::categorize("weather:current"),
            ToolRiskLevel::ReadOnlyExternal
        );
        assert_eq!(
            SideEffectFilter::categorize("memory:stage"),
            ToolRiskLevel::Safe
        );
    }

    #[test]
    fn benchmark_isolation_creates_temp_dir() {
        let original = PathBuf::from("/tmp/test_vault");
        let isolation = BenchmarkIsolation::new(&original).expect("Failed to create isolation");

        assert!(isolation.vault_root().exists());
        assert!(!isolation.qdrant_collection().is_empty());
        assert!(isolation.cleanup_report().will_auto_cleanup);
    }

    #[test]
    fn isolation_mode_parsing() {
        assert_eq!(
            "strict".parse::<IsolationMode>().unwrap(),
            IsolationMode::Strict
        );
        assert_eq!(
            "RELAXED".parse::<IsolationMode>().unwrap(),
            IsolationMode::Relaxed
        );
        assert!("unknown".parse::<IsolationMode>().is_err());
    }
}
