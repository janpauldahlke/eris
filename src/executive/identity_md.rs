//! Sync `00_Core/Identity.md` with `AppConfig::user_name` (fcp.toml is source of truth).

use std::path::Path;

use crate::executive::error::{FcpError, Result};

const USER_LINE_PREFIX: &str = "User Name is:";

/// Updates or removes the `User Name is:` line to match `user_name` from config.
/// Preserves all other lines, including `Agent Name:`.
pub async fn sync_identity_user_line(workspace_root: &Path, user_name: &str) -> Result<()> {
    let path = workspace_root.join("00_Core/Identity.md");
    let trimmed = user_name.trim();

    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(
                path = %path.display(),
                "Identity.md missing; skipping user name sync"
            );
            return Ok(());
        }
        Err(e) => {
            return Err(FcpError::Config(format!(
                "read {}: {}",
                path.display(),
                e
            )));
        }
    };

    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    lines.retain(|line| !line.trim_start().starts_with(USER_LINE_PREFIX));

    let mut inserted = false;
    let mut new_lines: Vec<String> = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        new_lines.push(line.clone());
        if !inserted && line.trim_start().starts_with("Agent Name:") {
            if !trimmed.is_empty() {
                new_lines.push(format!(
                    "User Name is: {} (your main user!)",
                    trimmed
                ));
            }
            inserted = true;
        }
    }

    if !trimmed.is_empty() && !inserted {
        tracing::warn!(
            "Identity.md has no 'Agent Name:' line; appending user name at end"
        );
        new_lines.push(format!(
            "User Name is: {} (your main user!)",
            trimmed
        ));
    }

    let mut out = new_lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }

    tokio::fs::write(&path, out.as_bytes())
        .await
        .map_err(|e| FcpError::Config(format!("write {}: {}", path.display(), e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn sync_inserts_after_agent_line() {
        let dir = tempdir().unwrap();
        let core = dir.path().join("00_Core");
        std::fs::create_dir_all(&core).unwrap();
        let path = core.join("Identity.md");
        std::fs::write(
            &path,
            "Intro line.\n\nAgent Name: Bot (this is you!)\n\nFooter.\n",
        )
        .unwrap();

        sync_identity_user_line(dir.path(), "Alice").await.unwrap();

        let got = std::fs::read_to_string(&path).unwrap();
        assert!(got.contains("Agent Name: Bot"));
        assert!(got.contains("User Name is: Alice (your main user!)"));
        assert!(got.contains("Footer."));
    }

    #[tokio::test]
    async fn sync_replaces_existing_user_line() {
        let dir = tempdir().unwrap();
        let core = dir.path().join("00_Core");
        std::fs::create_dir_all(&core).unwrap();
        let path = core.join("Identity.md");
        std::fs::write(
            &path,
            "Agent Name: X (this is you!)\nUser Name is: Old (your main user!)\n",
        )
        .unwrap();

        sync_identity_user_line(dir.path(), "New").await.unwrap();

        let got = std::fs::read_to_string(&path).unwrap();
        assert!(!got.contains("Old"));
        assert!(got.contains("User Name is: New (your main user!)"));
    }

    #[tokio::test]
    async fn sync_empty_removes_user_line() {
        let dir = tempdir().unwrap();
        let core = dir.path().join("00_Core");
        std::fs::create_dir_all(&core).unwrap();
        let path = core.join("Identity.md");
        std::fs::write(
            &path,
            "Agent Name: X (this is you!)\nUser Name is: Bob (your main user!)\n",
        )
        .unwrap();

        sync_identity_user_line(dir.path(), "").await.unwrap();

        let got = std::fs::read_to_string(&path).unwrap();
        assert!(!got.contains("User Name is:"));
        assert!(got.contains("Agent Name:"));
    }
}
