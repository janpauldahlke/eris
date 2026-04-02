//! Strict identity file loading for chat startup (workspace integrity, not tools).

use std::path::Path;
use std::sync::Arc;

use crate::executive::error::{FcpError, Result};

/// Fallback text only for debounced reload failures (keep last snapshot); not used for strict startup.
pub const FALLBACK_IDENTITY_TEXT: &str = "You are E.R.I.S., an autonomous AI agent.";

/// Chat startup: `Identity.md` must exist and be non-empty after bootstrap.
pub async fn read_identity_markdown_strict(
    workspace_label: &str,
    identity_path: &Path,
) -> Result<Arc<str>> {
    match tokio::fs::read_to_string(identity_path).await {
        Ok(content) if !content.trim().is_empty() => Ok(Arc::from(content)),
        Ok(_) => Err(FcpError::WorkspaceFault {
            workspace: workspace_label.to_string(),
            reason: format!(
                "identity file is empty: {}",
                identity_path.display()
            ),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(FcpError::WorkspaceFault {
            workspace: workspace_label.to_string(),
            reason: format!(
                "missing identity file: {}",
                identity_path.display()
            ),
        }),
        Err(e) => Err(FcpError::Io(e)),
    }
}

/// Reload after FS notification: on failure, caller keeps previous snapshot.
pub async fn try_read_identity_for_reload(identity_path: &Path) -> Result<Arc<str>> {
    let content = tokio::fs::read_to_string(identity_path).await.map_err(FcpError::Io)?;
    if content.trim().is_empty() {
        return Err(FcpError::Config(format!(
            "identity file empty: {}",
            identity_path.display()
        )));
    }
    Ok(Arc::from(content))
}
