//! Orchestrates probes and blocking prompts for first-run setup.

use std::path::Path;

use crate::config::AppConfig;
use crate::executive::cli::Cli;
use crate::executive::error::{FcpError, Result};

use super::gate;
use super::hint::IgnitionWorkspaceHint;
use super::prompts;
use super::report;

/// Interactive first-run flow when `.fcp/seal` is missing (caller verifies).
pub async fn run_welder_before_chat(
    cli: &Cli,
    config: &AppConfig,
    workspace_root: &Path,
) -> Result<IgnitionWorkspaceHint> {
    if !gate::welder_enabled_by_env() {
        tracing::info!(target: "fcp.setup", "Welder skipped (ERIS_SKIP_SETUP=1)");
        return Ok(IgnitionWorkspaceHint::from_cli(cli, workspace_root));
    }
    if std::env::var("CI").ok().as_deref() == Some("true") {
        tracing::info!(target: "fcp.setup", "Welder skipped (CI=true)");
        return Ok(IgnitionWorkspaceHint::from_cli(cli, workspace_root));
    }
    if !gate::stdin_is_interactive() {
        tracing::info!(target: "fcp.setup", "Welder skipped (stdin not a TTY)");
        return Ok(IgnitionWorkspaceHint::from_cli(cli, workspace_root));
    }

    let report = report::gather(config).await;
    let cli = cli.clone();
    let root = workspace_root.to_path_buf();
    let hint = tokio::task::spawn_blocking(move || {
        prompts::run_interactive_sequence(&report, &root, &cli)
    })
    .await
    .map_err(|e| FcpError::Config(format!("Welder task join: {e}")))?;
    hint
}
