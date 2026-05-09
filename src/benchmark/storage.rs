//! Benchmark result storage and retrieval.
//!
//! Manages saving benchmark reports to `.fcp/benchmarks/` and
//! loading them for comparison and trend analysis.

use crate::benchmark::metrics::BenchmarkReport;
use crate::executive::error::{FcpError, Result};
use std::path::{Path, PathBuf};
use tracing;

/// Run IDs embed `model_name`, which often looks like `org/model:tag`. Those characters must not
/// appear in a single filename: `PathBuf::join("x/y.json")` creates a subdirectory `x/` (ENOENT if
/// missing). Replace path-special characters so one report is always one file under `.fcp/benchmarks/`.
pub fn sanitize_run_id_for_path(run_id: &str) -> String {
    run_id
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '<' | '>' | '|' | '?' | '*' | '"' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn report_json_path(storage_dir: &Path, run_id: &str) -> PathBuf {
    let safe = sanitize_run_id_for_path(run_id);
    storage_dir.join(format!("{safe}.json"))
}

/// Load a [`BenchmarkReport`] from any readable JSON file (e.g. another vault’s `.fcp/benchmarks/*.json`).
pub fn load_report_from_file(path: &Path) -> Result<BenchmarkReport> {
    let json = std::fs::read_to_string(path).map_err(|e| {
        FcpError::Io(std::io::Error::other(format!(
            "Failed to read benchmark report {}: {e}",
            path.display()
        )))
    })?;

    serde_json::from_str(&json).map_err(|e| {
        FcpError::Config(format!(
            "Failed to parse benchmark report {}: {e}",
            path.display()
        ))
    })
}

/// Storage manager for benchmark results.
pub struct BenchmarkStorage {
    /// Root directory for benchmark storage (typically `.fcp/benchmarks/`).
    storage_dir: PathBuf,
}

impl BenchmarkStorage {
    /// Create a new storage manager for a vault.
    pub fn for_vault(vault_root: &Path) -> Result<Self> {
        let storage_dir = vault_root.join(".fcp").join("benchmarks");

        // Ensure directory exists
        std::fs::create_dir_all(&storage_dir).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to create benchmarks directory: {}", e),
            ))
        })?;

        tracing::debug!(
            storage_dir = %storage_dir.display(),
            "BenchmarkStorage: initialized"
        );

        Ok(Self { storage_dir })
    }

    /// Save a benchmark report.
    pub fn save_report(&self, report: &BenchmarkReport) -> Result<PathBuf> {
        let filepath = report_json_path(&self.storage_dir, &report.run_id);

        let json = serde_json::to_string_pretty(report).map_err(|e| {
            FcpError::Config(format!("Failed to serialize report: {}", e))
        })?;

        std::fs::write(&filepath, json).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to write report: {}", e),
            ))
        })?;

        // Create/update "latest.json" symlink/pointer
        let latest_path = self.storage_dir.join("latest.json");
        let _ = std::fs::remove_file(&latest_path); // Ignore if doesn't exist
        
        #[cfg(unix)]
        std::os::unix::fs::symlink(&filepath, &latest_path).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to create latest symlink: {}", e),
            ))
        })?;

        #[cfg(not(unix))]
        std::fs::write(&latest_path, filepath.to_string_lossy().as_bytes()).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to write latest pointer: {}", e),
            ))
        })?;

        tracing::info!(
            filepath = %filepath.display(),
            "BenchmarkStorage: saved report"
        );

        Ok(filepath)
    }

    /// Load a specific report by run ID.
    pub fn load_report(&self, run_id: &str) -> Result<BenchmarkReport> {
        let filepath = report_json_path(&self.storage_dir, run_id);
        self.load_report_from_path(&filepath)
    }

    /// Load a report from a specific path.
    pub fn load_report_from_path(&self, path: &Path) -> Result<BenchmarkReport> {
        let json = std::fs::read_to_string(path).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to read report from {}: {}", path.display(), e),
            ))
        })?;

        let report: BenchmarkReport = serde_json::from_str(&json).map_err(|e| {
            FcpError::Config(format!("Failed to parse report: {}", e))
        })?;

        tracing::debug!(
            path = %path.display(),
            model = %report.model_name,
            "BenchmarkStorage: loaded report"
        );

        Ok(report)
    }

    /// Load the latest report.
    pub fn load_latest(&self) -> Result<BenchmarkReport> {
        let latest_path = self.storage_dir.join("latest.json");

        // On Windows, latest.json contains the path
        #[cfg(not(unix))]
        {
            let target_path_str = std::fs::read_to_string(&latest_path).map_err(|e| {
                FcpError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to read latest pointer: {}", e),
                ))
            })?;
            let target_path = PathBuf::from(target_path_str.trim());
            return self.load_report_from_path(&target_path);
        }

        // On Unix, latest.json is a symlink
        #[cfg(unix)]
        {
            let target_path = std::fs::read_link(&latest_path).map_err(|e| {
                FcpError::Io(std::io::Error::other(
                    format!("Failed to read latest symlink: {}", e),
                ))
            })?;
            self.load_report_from_path(&target_path)
        }
    }

    /// List all available reports.
    pub fn list_reports(&self) -> Result<Vec<ReportInfo>> {
        let mut reports = Vec::new();

        let entries = std::fs::read_dir(&self.storage_dir).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to read benchmarks directory: {}", e),
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                FcpError::Io(std::io::Error::other(
                    format!("Failed to read directory entry: {}", e),
                ))
            })?;

            let path = entry.path();
            let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

            // Skip latest.json and non-JSON files
            if filename == "latest" || path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            // Try to load metadata without full parse
            match self.load_report(filename) {
                Ok(report) => {
                    reports.push(ReportInfo {
                        run_id: report.run_id.clone(),
                        model_name: report.model_name.clone(),
                        suite: report.suite.clone(),
                        timestamp: report.timestamp,
                        quality_score: report.quality.overall_quality_score(),
                        path: path.clone(),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to load report metadata"
                    );
                }
            }
        }

        // Sort by timestamp (newest first)
        reports.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(reports)
    }

    /// Get reports for comparison.
    pub fn get_comparison_reports(
        &self,
        baseline_run_id: &str,
        current_run_id: &str,
    ) -> Result<(BenchmarkReport, BenchmarkReport)> {
        let baseline = self.load_report(baseline_run_id)?;
        let current = self.load_report(current_run_id)?;
        Ok((baseline, current))
    }

    /// Get the last N reports for trend analysis.
    pub fn get_trend_reports(&self, count: usize) -> Result<Vec<BenchmarkReport>> {
        let all_reports = self.list_reports()?;
        let selected: Vec<_> = all_reports.into_iter().take(count).collect();

        let mut reports = Vec::new();
        for info in selected {
            match self.load_report(&info.run_id) {
                Ok(report) => reports.push(report),
                Err(e) => {
                    tracing::warn!(
                        run_id = %info.run_id,
                        error = %e,
                        "Failed to load report for trend analysis"
                    );
                }
            }
        }

        // Ensure chronological order (oldest first for trends)
        reports.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        Ok(reports)
    }

    /// Delete a specific report.
    pub fn delete_report(&self, run_id: &str) -> Result<()> {
        let filepath = report_json_path(&self.storage_dir, run_id);
        
        std::fs::remove_file(&filepath).map_err(|e| {
            FcpError::Io(std::io::Error::other(
                format!("Failed to delete report: {}", e),
            ))
        })?;

        tracing::info!(
            run_id = %run_id,
            "BenchmarkStorage: deleted report"
        );

        Ok(())
    }

    /// Clean up old reports, keeping only the most recent N.
    pub fn cleanup_old_reports(&self, keep_count: usize) -> Result<usize> {
        let all_reports = self.list_reports()?;
        
        if all_reports.len() <= keep_count {
            return Ok(0);
        }

        let to_delete = &all_reports[keep_count..];
        let mut deleted = 0;

        for info in to_delete {
            match self.delete_report(&info.run_id) {
                Ok(_) => deleted += 1,
                Err(e) => {
                    tracing::warn!(
                        run_id = %info.run_id,
                        error = %e,
                        "Failed to delete old report"
                    );
                }
            }
        }

        tracing::info!(
            deleted = deleted,
            kept = keep_count,
            "BenchmarkStorage: cleaned up old reports"
        );

        Ok(deleted)
    }

    /// Get storage directory path.
    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }
}

/// Summary information about a stored report.
#[derive(Debug, Clone)]
pub struct ReportInfo {
    pub run_id: String,
    pub model_name: String,
    pub suite: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub quality_score: f64,
    pub path: PathBuf,
}

impl ReportInfo {
    /// Format for display in lists.
    pub fn format_for_list(&self) -> String {
        format!(
            "{}  {:20}  {:12}  {:5.1}%  {}",
            self.timestamp.format("%Y-%m-%d %H:%M"),
            self.model_name.chars().take(20).collect::<String>(),
            self.suite,
            self.quality_score,
            self.run_id
        )
    }
}

/// Parse run IDs from diff argument (e.g., "run1..run2").
pub fn parse_diff_argument(arg: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = arg.split("..").collect();
    
    if parts.len() != 2 {
        return Err(FcpError::Config(
            "Diff argument must be in format 'run-id-1..run-id-2'".to_string()
        ));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Find a report by partial ID or date.
pub fn find_report_by_partial(storage: &BenchmarkStorage, partial: &str) -> Result<ReportInfo> {
    let reports = storage.list_reports()?;
    
    // Try exact match first
    if let Some(info) = reports.iter().find(|r| r.run_id == partial) {
        return Ok(info.clone());
    }

    // Try prefix match
    let matches: Vec<_> = reports
        .iter()
        .filter(|r| r.run_id.starts_with(partial))
        .collect();

    if matches.len() == 1 {
        return Ok(matches[0].clone());
    } else if matches.len() > 1 {
        return Err(FcpError::Config(format!(
            "Multiple reports match '{}': {:?}",
            partial,
            matches.iter().map(|m| &m.run_id).collect::<Vec<_>>()
        )));
    }

    // Try date match (YYYY-MM-DD)
    if partial.len() == 10 && partial.contains('-') {
        let date_matches: Vec<_> = reports
            .iter()
            .filter(|r| r.timestamp.format("%Y-%m-%d").to_string() == partial)
            .collect();

        if !date_matches.is_empty() {
            // Return most recent from that date
            return Ok(date_matches[0].clone());
        }
    }

    Err(FcpError::Config(format!(
        "No report found matching '{}'",
        partial
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::metrics::{
        BenchmarkReport, CleanupConfirmation, QualityMetrics, SpeedMetrics,
    };

    fn create_temp_storage() -> (tempfile::TempDir, BenchmarkStorage) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let storage = BenchmarkStorage::for_vault(temp_dir.path()).expect("storage");
        (temp_dir, storage)
    }

    fn create_test_report(name: &str) -> BenchmarkReport {
        BenchmarkReport {
            run_id: format!("{}_{}", chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S"), name),
            timestamp: chrono::Utc::now(),
            model_name: name.to_string(),
            suite: "quick".to_string(),
            quality: QualityMetrics::default(),
            speed: SpeedMetrics::default(),
            isolation_mode: "Strict".to_string(),
            cleanup_report: CleanupConfirmation::default(),
        }
    }

    #[test]
    fn storage_creates_directory() {
        let (temp_dir, storage) = create_temp_storage();
        
        assert!(storage.storage_dir().exists());
        assert!(storage.storage_dir().to_string_lossy().contains("benchmarks"));
        
        // temp_dir must be kept alive for the duration
        let _ = temp_dir;
    }

    #[test]
    fn save_and_load_report() {
        let (temp_dir, storage) = create_temp_storage();
        
        let report = create_test_report("test-model");
        let saved_path = storage.save_report(&report).expect("save");
        
        assert!(saved_path.exists());
        
        let loaded = storage.load_report(&report.run_id).expect("load");
        assert_eq!(loaded.model_name, report.model_name);
        
        let _ = temp_dir;
    }

    #[test]
    fn list_reports_returns_results() {
        let (temp_dir, storage) = create_temp_storage();
        
        // Save multiple reports
        for i in 0..3 {
            let report = create_test_report(&format!("model-{}", i));
            storage.save_report(&report).expect("save");
        }
        
        let list = storage.list_reports().expect("list");
        assert_eq!(list.len(), 3);
        
        // Should be sorted by timestamp (newest first)
        for i in 0..list.len() - 1 {
            assert!(list[i].timestamp >= list[i + 1].timestamp);
        }
        
        let _ = temp_dir;
    }

    #[test]
    fn sanitize_run_id_prevents_nested_paths() {
        let raw = "2026-01-01_12-00-00_org/model:name";
        let safe = sanitize_run_id_for_path(raw);
        assert!(!safe.contains('/'));
        assert!(!safe.contains(':'));
        assert!(safe.contains("org_model"));
    }

    #[test]
    fn parse_diff_argument_works() {
        let result = parse_diff_argument("run1..run2").expect("parse");
        assert_eq!(result, ("run1".to_string(), "run2".to_string()));
    }

    #[test]
    fn parse_diff_argument_invalid() {
        let result = parse_diff_argument("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn cleanup_keeps_recent_reports() {
        let (temp_dir, storage) = create_temp_storage();
        
        // Save 5 reports
        for i in 0..5 {
            let report = create_test_report(&format!("model-{}", i));
            storage.save_report(&report).expect("save");
            std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure different timestamps
        }
        
        // Cleanup, keeping only 2
        let deleted = storage.cleanup_old_reports(2).expect("cleanup");
        assert_eq!(deleted, 3);
        
        let remaining = storage.list_reports().expect("list");
        assert_eq!(remaining.len(), 2);
        
        let _ = temp_dir;
    }

    #[test]
    fn report_info_formats_nicely() {
        let info = ReportInfo {
            run_id: "2024-01-01_12-00-00_test".to_string(),
            model_name: "gemma4-26b".to_string(),
            suite: "standard".to_string(),
            timestamp: chrono::Utc::now(),
            quality_score: 95.5,
            path: PathBuf::from("/tmp/test.json"),
        };

        let formatted = info.format_for_list();
        assert!(formatted.contains("gemma4-26b"));
        assert!(formatted.contains("95.5"));
        assert!(formatted.contains("standard"));
    }
}
