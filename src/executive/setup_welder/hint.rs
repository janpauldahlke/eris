//! Workspace id chosen or inferred for first ignition.

use std::path::Path;

use crate::executive::cli::Cli;

use super::prompts;

/// Workspace id written into fresh `.fcp/config.toml` after ignition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnitionWorkspaceHint {
    pub workspace: String,
}

impl IgnitionWorkspaceHint {
    /// When the welder is skipped, derive from CLI (`-w`) or the current folder name.
    pub fn from_cli(cli: &Cli, workspace_root: &Path) -> Self {
        let folder = workspace_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("vault");
        let workspace = if cli.workspace != "default" {
            prompts::sanitize_workspace_id(&cli.workspace)
        } else {
            prompts::sanitize_workspace_id(folder)
        };
        Self { workspace }
    }
}
