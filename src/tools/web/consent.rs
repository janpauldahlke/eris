//! Deterministic consent-banner handling via browser39 link-by-text clicks.

use crate::executive::error::{FcpError, Result};
use crate::tools::web::fetcher::{
    browser39_run_batch_blocking, parse_browser39_fetch_line, FetchedPage,
};
use crate::tools::web::ledger::normalize_host;
use crate::vault_layout;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Per-host CMP button labels (vault `.fcp/browser39/consent_profiles.toml`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HostConsentProfile {
    pub host: String,
    #[serde(default)]
    pub accept_link_text: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ConsentProfilesFile {
    #[serde(default)]
    host: Vec<HostConsentProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentOutcome {
    pub attempted: bool,
    pub improved: bool,
    /// True when every accept-label click returned ok=false (link not in static HTML).
    pub all_clicks_failed: bool,
}

const GENERIC_ACCEPT_LABELS: &[&str] = &[
    "Alle akzeptieren",
    "Accept all",
    "Accept All",
    "Zustimmen",
    "Akzeptieren",
    "I agree",
    "Yes, I agree",
    "Allow all",
    "Allow All",
    "Agree",
    "Einverstanden",
];

pub fn consent_profiles_path(vault_root: &Path) -> PathBuf {
    vault_layout::fcp_dir(vault_root).join("browser39/consent_profiles.toml")
}

pub fn load_consent_profiles(vault_root: &Path) -> Result<Vec<HostConsentProfile>> {
    let path = consent_profiles_path(vault_root);
    if !path.is_file() {
        return Ok(default_profiles());
    }
    let raw = std::fs::read_to_string(&path).map_err(FcpError::Io)?;
    let file: ConsentProfilesFile = toml::from_str(&raw).map_err(|e| {
        FcpError::Config(format!(
            "invalid consent profiles {}: {e}",
            path.display()
        ))
    })?;
    if file.host.is_empty() {
        return Ok(default_profiles());
    }
    Ok(file.host)
}

fn default_profiles() -> Vec<HostConsentProfile> {
    vec![
        profile("kicker.de", &["Alle akzeptieren", "Accept all", "Zustimmen", "Akzeptieren"]),
        profile("gamestar.de", &["Alle akzeptieren", "Accept all", "Zustimmen", "I agree"]),
        profile("bbc.com", &["Yes, I agree", "Allow all", "Accept"]),
        profile("spiegel.de", &["Alle akzeptieren", "Akzeptieren", "Zustimmen"]),
        profile("taz.de", &["Alle akzeptieren", "Akzeptieren", "Zustimmen"]),
    ]
}

fn profile(host: &str, labels: &[&str]) -> HostConsentProfile {
    HostConsentProfile {
        host: host.to_string(),
        accept_link_text: labels.iter().map(|s| (*s).to_string()).collect(),
    }
}

pub fn accept_texts_for_host(profiles: &[HostConsentProfile], host: &str) -> Vec<String> {
    let key = normalize_host(host);
    let mut out: Vec<String> = profiles
        .iter()
        .find(|p| normalize_host(&p.host) == key)
        .map(|p| p.accept_link_text.clone())
        .unwrap_or_default();
    for label in GENERIC_ACCEPT_LABELS {
        if !out.iter().any(|s| s.eq_ignore_ascii_case(label)) {
            out.push((*label).to_string());
        }
    }
    out
}

pub fn host_session_dir(vault_root: &Path, host: &str) -> PathBuf {
    let key = normalize_host(host);
    vault_layout::fcp_dir(vault_root)
        .join("browser39/sessions/hosts")
        .join(&key)
}

/// Fetch with optional in-batch consent clicks (shared browser39 session per batch).
pub fn fetch_with_consent_blocking(
    binary: &str,
    config_path: &Path,
    session_dir: &Path,
    persist_sessions: bool,
    url: &str,
    selector: Option<&str>,
    max_tokens: u32,
    offset: u32,
    accept_texts: &[String],
    max_attempts: u32,
    thin_threshold: usize,
) -> Result<(FetchedPage, ConsentOutcome)> {
    // Host sessions need disk persist between batch invocations so consent clicks see the CMP page.
    let no_persist = !persist_sessions;

    let initial_cmds = vec![fetch_url_command(
        "initial",
        1,
        url,
        selector,
        max_tokens,
        offset,
    )];
    let initial_lines = browser39_run_batch_blocking(
        binary,
        config_path,
        session_dir,
        no_persist,
        &initial_cmds,
    )?;
    let initial = parse_first_fetch_line(&initial_lines)?;
    let initial_chars = initial.markdown.chars().count();

    if initial_chars >= thin_threshold || max_attempts == 0 || accept_texts.is_empty() {
        return Ok((
            initial,
            ConsentOutcome {
                attempted: false,
                improved: false,
                all_clicks_failed: false,
            },
        ));
    }

    info!(
        event = "web.consent.thin_page",
        url = %url,
        markdown_chars = initial_chars,
        threshold = thin_threshold,
        "attempting consent link clicks"
    );

    let mut best = initial;
    let mut attempts = 0u32;
    let mut click_failures = 0u32;
    for text in accept_texts.iter().take(max_attempts as usize) {
        attempts = attempts.saturating_add(1);
        debug!(
            event = "web.consent.accept_attempt",
            url = %url,
            label = %text,
            attempt = attempts,
            "browser39 click by link text"
        );
        let cmds = vec![
            click_text_command(&format!("accept-{attempts}"), attempts * 2, text),
            fetch_url_command(
                &format!("refetch-{attempts}"),
                attempts * 2 + 1,
                url,
                selector,
                max_tokens,
                offset,
            ),
        ];
        let lines = match browser39_run_batch_blocking(
            binary,
            config_path,
            session_dir,
            no_persist,
            &cmds,
        ) {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    event = "web.consent.accept_failed",
                    url = %url,
                    label = %text,
                    error = %e,
                    "consent batch failed"
                );
                continue;
            }
        };
        if !click_line_succeeded(lines.first()) {
            click_failures = click_failures.saturating_add(1);
            warn!(
                event = "web.consent.click_failed",
                url = %url,
                label = %text,
                attempt = attempts,
                "browser39 click did not succeed (link text not found or ok=false)"
            );
        }
        let Some(refetched) = parse_last_fetch_line(&lines) else {
            warn!(
                event = "web.consent.refetch_missing",
                url = %url,
                label = %text,
                "consent batch had no refetch result line"
            );
            continue;
        };
        let refetched_chars = refetched.markdown.chars().count();
        if refetched_chars > best.markdown.chars().count() {
            info!(
                event = "web.consent.refetch_ok",
                url = %url,
                label = %text,
                markdown_chars = refetched_chars,
                "consent refetch improved body"
            );
            best = refetched;
        } else {
            debug!(
                event = "web.consent.refetch_no_improvement",
                url = %url,
                label = %text,
                markdown_chars = refetched_chars,
                "consent refetch did not improve body"
            );
        }
        if refetched_chars >= thin_threshold {
            break;
        }
    }

    let improved = best.markdown.chars().count() > initial_chars;
    Ok((
        best,
        ConsentOutcome {
            attempted: true,
            improved,
            all_clicks_failed: attempts > 0 && click_failures >= attempts,
        },
    ))
}

fn fetch_url_command(
    id: &str,
    seq: u32,
    url: &str,
    selector: Option<&str>,
    max_tokens: u32,
    offset: u32,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "action": "fetch",
        "v": 1,
        "seq": seq,
        "url": url,
        "options": fetch_options(selector, max_tokens, offset),
    })
}

fn click_text_command(id: &str, seq: u32, text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "action": "click",
        "v": 1,
        "seq": seq,
        "text": text,
    })
}

fn click_line_succeeded(line: Option<&String>) -> bool {
    let Some(line) = line else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    value.get("ok").and_then(|v| v.as_bool()) != Some(false)
}

fn fetch_options(selector: Option<&str>, max_tokens: u32, offset: u32) -> serde_json::Value {
    let mut options = serde_json::json!({
        "max_tokens": max_tokens,
        "offset": offset,
        "include_links": true,
        "strip_nav": true,
        "show_selectors_first": false,
    });
    if let Some(sel) = selector.filter(|s| !s.trim().is_empty()) {
        options["selector"] = serde_json::Value::String(sel.to_string());
    }
    options
}

fn parse_first_fetch_line(lines: &[String]) -> Result<FetchedPage> {
    lines
        .iter()
        .find_map(|line| parse_browser39_fetch_line(line).ok())
        .ok_or_else(|| FcpError::ToolFault {
            tool_name: "web:fetch".into(),
            reason: "browser39 consent batch: no fetch result".into(),
        })
}

fn parse_last_fetch_line(lines: &[String]) -> Option<FetchedPage> {
    lines
        .iter()
        .rev()
        .find_map(|line| parse_browser39_fetch_line(line).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_host_profile_lookup() {
        let profiles = default_profiles();
        let texts = accept_texts_for_host(&profiles, "www.kicker.de");
        assert!(texts.iter().any(|t| t.contains("akzeptieren")));
    }

    #[test]
    fn generic_labels_appended_for_unknown_host() {
        let texts = accept_texts_for_host(&[], "example.org");
        assert!(texts.iter().any(|t| t == "Accept all"));
    }

    #[test]
    fn fetch_options_include_selector() {
        let opts = fetch_options(Some("article"), 1000, 0);
        assert_eq!(opts["selector"], "article");
    }

    #[test]
    fn click_command_uses_click_action() {
        let cmd = click_text_command("c1", 2, "Accept all");
        assert_eq!(cmd["action"], "click");
        assert_eq!(cmd["text"], "Accept all");
    }

    #[test]
    fn click_line_succeeded_reads_ok_flag() {
        assert!(click_line_succeeded(Some(
            &r#"{"ok":true,"result":{}}"#.to_string()
        )));
        assert!(!click_line_succeeded(Some(
            &r#"{"ok":false,"error":"LINK_NOT_FOUND"}"#.to_string()
        )));
    }
}
