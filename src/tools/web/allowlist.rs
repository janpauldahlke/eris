//! Glob allowlist at `{vault}/.fcp/web_allowlist.toml`.

use crate::executive::error::{FcpError, Result};
use crate::vault_layout;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WebAllowlistFile {
    #[serde(default)]
    pub patterns: Vec<String>,
}

impl Default for WebAllowlistFile {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }
}

pub fn allowlist_path(vault_root: &Path, override_path: Option<&Path>) -> PathBuf {
    override_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| vault_layout::fcp_dir(vault_root).join("web_allowlist.toml"))
}

pub fn load_allowlist(vault_root: &Path, override_path: Option<&Path>) -> Result<WebAllowlistFile> {
    let path = allowlist_path(vault_root, override_path);
    if !path.is_file() {
        return Ok(WebAllowlistFile::default());
    }
    let raw = std::fs::read_to_string(&path).map_err(FcpError::Io)?;
    let mut file: WebAllowlistFile = toml::from_str(&raw).map_err(|e| {
        FcpError::Config(format!(
            "invalid web allowlist {}: {e}",
            path.display()
        ))
    })?;
    for pattern in &mut file.patterns {
        *pattern = pattern.trim().to_string();
    }
    Ok(file)
}

/// Enforce allowlist when `enabled`; no-op when disabled (operator toggle in `[web].allowlist_enabled`).
pub fn enforce_allowlist(
    enabled: bool,
    url: &str,
    allowlist: &WebAllowlistFile,
) -> Result<()> {
    if enabled {
        require_allowed(url, allowlist)
    } else {
        Ok(())
    }
}

pub fn require_allowed(url: &str, allowlist: &WebAllowlistFile) -> Result<()> {
    if allowlist.patterns.is_empty() {
        return Err(FcpError::PolicyViolation {
            code: "WEB_ALLOWLIST_EMPTY".to_string(),
            message: "no patterns in .fcp/web_allowlist.toml — add glob patterns before fetching"
                .to_string(),
        });
    }
    if allowlist
        .patterns
        .iter()
        .any(|p| glob_match(p, url))
    {
        Ok(())
    } else {
        Err(FcpError::PolicyViolation {
            code: "WEB_ALLOWLIST_DENIED".to_string(),
            message: format!("URL not covered by web_allowlist patterns: {}", url.trim()),
        })
    }
}

/// Trim whitespace and canonicalize `http(s)` URLs for allowlist comparison.
pub fn normalize_allowlist_token(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let Ok(mut url) = Url::parse(raw) else {
        return collapse_slashes(raw.trim_end_matches('/'));
    };
    if !matches!(url.scheme(), "http" | "https") {
        return collapse_slashes(raw.trim_end_matches('/'));
    }
    url.set_fragment(None);
    let path = collapse_path_slashes(url.path());
    url.set_path(if path.is_empty() { "/" } else { &path });
    let mut s = url.to_string();
    if path.is_empty() || path == "/" {
        s = s.trim_end_matches('/').to_string();
    } else {
        s = s.trim_end_matches('/').to_string();
    }
    s
}

fn collapse_slashes(s: &str) -> String {
    let s = s.trim();
    if !s.contains("//") {
        return s.trim_end_matches('/').to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for ch in s.chars() {
        if ch == '/' {
            if !prev_slash {
                out.push(ch);
            }
            prev_slash = true;
        } else {
            out.push(ch);
            prev_slash = false;
        }
    }
    out.trim_end_matches('/').to_string()
}

/// Collapse `//` in a URL path; drop a lone `/` (origin-only).
fn collapse_path_slashes(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return String::new();
    }
    format!("/{}", segments.join("/"))
}

/// Match `pattern` against full URL (supports trailing `**` path wildcards).
pub fn glob_match(pattern: &str, url: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    let url_norm = normalize_allowlist_token(url);
    if url_norm.is_empty() {
        return false;
    }
    if let Some(prefix) = pattern.strip_suffix("**") {
        let prefix_norm = normalize_allowlist_token(prefix);
        if prefix_norm.is_empty() {
            return true;
        }
        return url_norm == prefix_norm || url_norm.starts_with(&format!("{prefix_norm}/"));
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        let prefix_norm = normalize_allowlist_token(prefix);
        return !prefix_norm.is_empty() && url_norm.starts_with(&prefix_norm);
    }
    let pattern_norm = normalize_allowlist_token(pattern);
    url_norm == pattern_norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_allowlist_skips_when_disabled() {
        let list = WebAllowlistFile {
            patterns: vec!["https://example.com/**".into()],
        };
        assert!(enforce_allowlist(false, "https://other.test/", &list).is_ok());
        assert!(enforce_allowlist(true, "https://other.test/", &list).is_err());
    }

    #[test]
    fn glob_double_star_matches_path_tail() {
        let list = WebAllowlistFile {
            patterns: vec!["https://www.bbc.com/news/**".into()],
        };
        assert!(require_allowed(
            "https://www.bbc.com/news/world-123",
            &list
        )
        .is_ok());
        assert!(require_allowed("https://www.bbc.com/", &list).is_err());
    }

    #[test]
    fn exact_pattern_ignores_trailing_slash() {
        let list = WebAllowlistFile {
            patterns: vec!["https://www.bbc.com/".into()],
        };
        assert!(require_allowed("https://www.bbc.com", &list).is_ok());
        assert!(require_allowed("https://www.bbc.com/", &list).is_ok());
        assert!(require_allowed("  https://www.bbc.com/  ", &list).is_ok());
    }

    #[test]
    fn double_star_matches_origin_without_path() {
        let list = WebAllowlistFile {
            patterns: vec!["https://taz.de/**".into()],
        };
        assert!(require_allowed("https://taz.de", &list).is_ok());
        assert!(require_allowed("https://taz.de/artikel", &list).is_ok());
    }

    #[test]
    fn collapses_leading_slashes_in_path() {
        let list = WebAllowlistFile {
            patterns: vec!["https://www.bbc.com/news/**".into()],
        };
        assert!(require_allowed(
            "https://www.bbc.com//news/world",
            &list
        )
        .is_ok());
    }

    #[test]
    fn trims_pattern_whitespace() {
        let list = WebAllowlistFile {
            patterns: vec!["  https://www.bbc.com/  ".into()],
        };
        assert!(require_allowed("https://www.bbc.com", &list).is_ok());
    }

    #[test]
    fn empty_patterns_policy_error() {
        let err = require_allowed("https://example.com/", &WebAllowlistFile::default())
            .expect_err("empty");
        assert_eq!(err.to_string(), format!(
            "Policy violation [WEB_ALLOWLIST_EMPTY]: no patterns in .fcp/web_allowlist.toml — add glob patterns before fetching"
        ));
    }
}
