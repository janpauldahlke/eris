//! browser39 binary probe and vault operator files under `.fcp/browser39/`.

use crate::executive::error::{FcpError, Result};
use crate::tools::web::fetch_inner::FALLBACK_WEB_FETCH_UA;
use crate::vault_layout;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::fs;

const INSTALL_HINT: &str = "Install: cargo install browser39 --locked  (see docs/WEB_BROWSER39.md)";

/// Resolved browser39 executable (`BROWSER39_BIN` or `browser39`).
pub fn resolve_browser39_binary() -> String {
    std::env::var("BROWSER39_BIN").unwrap_or_else(|_| "browser39".into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser39ProbeOk {
    pub binary: String,
    pub version_line: String,
}

/// Verify `binary` exists and responds to `--version` (blocking; use from `spawn_blocking`).
pub fn probe_browser39_binary_sync(binary: &str) -> Result<Browser39ProbeOk> {
    let binary = binary.trim();
    if binary.is_empty() {
        return Err(FcpError::Config(
            "BROWSER39_BIN is empty; set it to the browser39 executable path".into(),
        ));
    }
    let output = Command::new(binary).arg("--version").output().map_err(|e| {
        FcpError::NetworkFault(format!(
            "browser39 binary {binary:?} not found or not executable: {e}. {INSTALL_HINT}"
        ))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        return Err(FcpError::NetworkFault(format!(
            "browser39 --version failed (exit {:?}): {detail}. {INSTALL_HINT}",
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version_line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("browser39")
        .trim()
        .to_string();
    Ok(Browser39ProbeOk {
        binary: binary.to_string(),
        version_line,
    })
}

/// Seed allowlist template and browser39 config stubs (idempotent).
pub async fn seed_web_operator_files(workspace_root: &Path) -> Result<()> {
    let allowlist = vault_layout::fcp_dir(workspace_root).join("web_allowlist.toml");
    if !allowlist.exists() {
        let body = r#"# Glob patterns — enable origins you fetch (article paths need wildcards).
patterns = [
  # "https://www.bbc.com/",
  # "https://www.bbc.com/news/**",
  # "https://en.wikipedia.org/wiki/**",
]
"#;
        fs::write(&allowlist, body).await?;
    }
    let b39_dir = vault_layout::fcp_dir(workspace_root).join("browser39");
    if !b39_dir.exists() {
        fs::create_dir_all(&b39_dir).await?;
    }
    let b39_cfg = b39_dir.join("config.toml");
    if !b39_cfg.exists() {
        let body = r#"# browser39 template — eris merges user_agent from .fcp/config.toml at chat bootstrap.
# [session]
# timeout_secs = 30
"#;
        fs::write(&b39_cfg, body).await?;
    }
    let consent_profiles = b39_dir.join("consent_profiles.toml");
    if !consent_profiles.exists() {
        let body = r#"# Host-specific consent button labels for browser39 `fetch` by link text.
# eris tries these when page markdown is below [web].thin_page_char_threshold.

[[host]]
host = "kicker.de"
accept_link_text = ["Alle akzeptieren", "Accept all", "Zustimmen", "Akzeptieren"]

[[host]]
host = "gamestar.de"
accept_link_text = ["Alle akzeptieren", "Accept all", "Zustimmen", "I agree"]

[[host]]
host = "bbc.com"
accept_link_text = ["Yes, I agree", "Allow all", "Accept"]

[[host]]
host = "spiegel.de"
accept_link_text = ["Alle akzeptieren", "Akzeptieren", "Zustimmen"]

[[host]]
host = "taz.de"
accept_link_text = ["Alle akzeptieren", "Akzeptieren", "Zustimmen"]
"#;
        fs::write(&consent_profiles, body).await?;
    }
    Ok(())
}

/// Write or merge `[user_agent]` in `{vault}/.fcp/browser39/config.toml` (blocking).
pub fn ensure_browser39_vault_config(vault_root: &Path, user_agent: &str) -> Result<PathBuf> {
    let path = vault_layout::fcp_dir(vault_root).join("browser39/config.toml");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(FcpError::Io)?;
    }
    let ua = user_agent.trim();
    let ua = if ua.is_empty() {
        FALLBACK_WEB_FETCH_UA
    } else {
        ua
    };
    let body = if path.is_file() {
        let existing = std::fs::read_to_string(&path).map_err(FcpError::Io)?;
        merge_user_agent_toml(&existing, ua)
    } else {
        format!(
            "# browser39 config (eris-managed user_agent)\n[user_agent]\nvalue = \"{ua}\"\n"
        )
    };
    std::fs::write(&path, body).map_err(FcpError::Io)?;
    Ok(path)
}

fn merge_user_agent_toml(existing: &str, user_agent: &str) -> String {
    let escaped = user_agent.replace('\\', "\\\\").replace('\"', "\\\"");
    if existing.contains("[user_agent]") {
        let mut out = String::new();
        let mut in_ua = false;
        let mut replaced = false;
        for line in existing.lines() {
            if line.trim() == "[user_agent]" {
                in_ua = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_ua && line.trim_start().starts_with("value") {
                out.push_str(&format!("value = \"{escaped}\"\n"));
                replaced = true;
                in_ua = false;
                continue;
            }
            if in_ua && line.starts_with('[') {
                in_ua = false;
            }
            out.push_str(line);
            out.push('\n');
        }
        if !replaced {
            out.push_str(&format!("\n[user_agent]\nvalue = \"{escaped}\"\n"));
        }
        out
    } else {
        format!("{existing}\n[user_agent]\nvalue = \"{escaped}\"\n")
    }
}

/// Chat bootstrap: optional binary probe, seed operator files, sync user-agent into browser39 config.
pub async fn ensure_web_stack_ready(
    vault_root: &Path,
    user_agent: &str,
    require_binary: bool,
) -> Result<Option<Browser39ProbeOk>> {
    seed_web_operator_files(vault_root).await?;

    let probe = if require_binary {
        let binary = resolve_browser39_binary();
        Some(
            tokio::task::spawn_blocking(move || probe_browser39_binary_sync(&binary))
                .await
                .map_err(|e| FcpError::EngineFault(format!("browser39 probe join: {e}")))??,
        )
    } else {
        None
    };

    let vault_root = vault_root.to_path_buf();
    let user_agent = user_agent.to_string();
    tokio::task::spawn_blocking(move || ensure_browser39_vault_config(&vault_root, &user_agent))
        .await
        .map_err(|e| FcpError::EngineFault(format!("browser39 config join: {e}")))??;

    Ok(probe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn probe_rejects_missing_binary() {
        let err = probe_browser39_binary_sync("/nonexistent/browser39-eris-test")
            .expect_err("missing binary");
        match err {
            FcpError::NetworkFault(msg) => {
                assert!(msg.contains("not found") || msg.contains("not executable"));
                assert!(msg.contains("cargo install browser39"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn seed_creates_operator_files() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(vault_layout::fcp_dir(root)).expect("fcp dir");
        seed_web_operator_files(root).await.expect("seed");
        assert!(vault_layout::fcp_dir(root).join("web_allowlist.toml").is_file());
        assert!(vault_layout::fcp_dir(root)
            .join("browser39/consent_profiles.toml")
            .is_file());
    }

    #[test]
    fn ensure_config_writes_user_agent() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let path = ensure_browser39_vault_config(root, "TestAgent/1.0").expect("config");
        let body = std::fs::read_to_string(path).expect("read");
        assert!(body.contains("TestAgent/1.0"));
    }
}
