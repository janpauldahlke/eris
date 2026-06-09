//! Merge operator-editable tool-family keys into `.fcp/config.toml`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::tools::registration::{
    DB_REST_API_PROFILES, WEATHER_API_PROFILES, WIKI_API_PROFILES,
};
use crate::vault_layout;

use super::tools_config_schema::family_field_keys;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolsUpdatePayload {
    pub family_id: String,
    #[serde(default)]
    pub values: BTreeMap<String, JsonValue>,
}

pub async fn merge_tools_into_toml(
    vault_root: &Path,
    _config: &AppConfig,
    payload: &ToolsUpdatePayload,
) -> Result<()> {
    let allowed: Vec<&str> = family_field_keys(&payload.family_id);
    if allowed.is_empty() {
        return Err(FcpError::Config(format!(
            "unknown or read-only tool family: {}",
            payload.family_id
        )));
    }

    let path = vault_layout::config_toml(vault_root);
    let raw = tokio::fs::read_to_string(&path).await.map_err(FcpError::Io)?;
    let mut doc: toml::Table = toml::from_str(&raw)
        .map_err(|e| FcpError::Config(format!("parse config.toml: {e}")))?;

    for (key, val) in &payload.values {
        if !allowed.iter().any(|k| k == key) {
            return Err(FcpError::Config(format!(
                "key {key} is not editable for family {}",
                payload.family_id
            )));
        }
        apply_key(&mut doc, key, val)?;
    }

    if payload.family_id == "weather" {
        let enabled = read_bool_key(&doc, "weather_enabled").unwrap_or(true);
        sync_api_profiles_enabled(&mut doc, WEATHER_API_PROFILES, enabled);
    }
    if payload.family_id == "wiki" {
        let enabled = read_bool_key(&doc, "wiki_enabled").unwrap_or(true);
        sync_api_profiles_enabled(&mut doc, WIKI_API_PROFILES, enabled);
    }
    if payload.family_id == "trains" {
        let enabled = read_bool_key(&doc, "db_rest_enabled").unwrap_or(true);
        sync_api_profiles_enabled(&mut doc, DB_REST_API_PROFILES, enabled);
    }

    let out = toml::to_string_pretty(&doc)
        .map_err(|e| FcpError::Config(format!("serialize config.toml: {e}")))?;
    tokio::fs::write(&path, out.as_bytes())
        .await
        .map_err(FcpError::Io)?;
    Ok(())
}

fn apply_key(doc: &mut toml::Table, key: &str, val: &JsonValue) -> Result<()> {
    match key {
        "vault_read_ratio" => {
            let f = json_f64(val)?;
            if !(0.05..=1.0).contains(&f) {
                return Err(FcpError::Config(
                    "vault_read_ratio must be between 0.05 and 1.0".into(),
                ));
            }
            doc.insert(key.to_string(), toml::Value::Float(f));
        }
        "vault_search_max_files" | "vault_search_max_total_chars" => {
            let n = json_usize(val)?;
            if n == 0 {
                return Err(FcpError::Config(format!("{key} must be positive")));
            }
            doc.insert(key.to_string(), toml::Value::Integer(n as i64));
        }
        "memory_query_default_top_k" | "memory_query_top_k_max" | "memory_query_default_max_total_chars" => {
            let n = json_usize(val)?;
            if n == 0 {
                return Err(FcpError::Config(format!("{key} must be positive")));
            }
            doc.insert(key.to_string(), toml::Value::Integer(n as i64));
        }
        "require_semantic_brain" | "news_today_enabled" | "weather_enabled" | "wiki_enabled"
        | "db_rest_enabled" => {
            doc.insert(key.to_string(), toml::Value::Boolean(json_bool(val)?));
        }
        "news_today_site_base" | "news_today_default_homepage" => {
            doc.insert(
                key.to_string(),
                toml::Value::String(json_string(val)?.trim().to_string()),
            );
        }
        "news_today_max_headlines_default" => {
            let n = json_usize(val)?;
            doc.insert(key.to_string(), toml::Value::Integer(n as i64));
        }
        "news_today_deep_fetch_max_default" => {
            let n = json_u64(val)?;
            if n > 3 {
                return Err(FcpError::Config(
                    "news_today_deep_fetch_max_default must be 0..3".into(),
                ));
            }
            doc.insert(key.to_string(), toml::Value::Integer(n as i64));
        }
        "web.search_enabled" => {
            set_nested_bool(doc, &["web"], "search_enabled", json_bool(val)?);
        }
        "web.default_fetch_budget" | "web.max_fetches_per_user_turn" | "web.max_web_tool_calls_per_turn" => {
            let n = json_u64(val)?;
            if n == 0 {
                return Err(FcpError::Config(format!("{key} must be positive")));
            }
            let field = key.strip_prefix("web.").expect("web prefix");
            set_nested_integer(doc, &["web"], field, n);
        }
        "web.require_find_before_refetch" | "web.allowlist_enabled" | "web.explore_site_enabled" => {
            let field = key.strip_prefix("web.").expect("web prefix");
            set_nested_bool(doc, &["web"], field, json_bool(val)?);
        }
        "google.enabled" => {
            set_nested_bool(doc, &["google"], "enabled", json_bool(val)?);
        }
        "google.service_account_key" => {
            let s = json_string(val)?.trim().to_string();
            if s.is_empty() {
                set_nested_remove(doc, &["google"], "service_account_key");
            } else {
                set_nested_string(doc, &["google"], "service_account_key", s);
            }
        }
        "google.impersonate_user" => {
            let s = json_string(val)?.trim().to_string();
            if s.is_empty() {
                set_nested_remove(doc, &["google"], "impersonate_user");
            } else {
                set_nested_string(doc, &["google"], "impersonate_user", s);
            }
        }
        "moltbook.enabled" => {
            set_nested_bool(doc, &["moltbook"], "enabled", json_bool(val)?);
        }
        "moltbook.api_key_file" => {
            let s = json_string(val)?.trim().to_string();
            if s.is_empty() {
                set_nested_remove(doc, &["moltbook"], "api_key_file");
            } else {
                set_nested_string(doc, &["moltbook"], "api_key_file", s);
            }
        }
        "moltbook.agent_name" => {
            let s = json_string(val)?.trim().to_string();
            if s.is_empty() {
                set_nested_remove(doc, &["moltbook"], "agent_name");
            } else {
                set_nested_string(doc, &["moltbook"], "agent_name", s);
            }
        }
        "moltbook.timeout_secs" => {
            let n = json_u64(val)?;
            if n == 0 {
                return Err(FcpError::Config("moltbook.timeout_secs must be positive".into()));
            }
            set_nested_integer(doc, &["moltbook"], "timeout_secs", n);
        }
        "moltbook.max_response_bytes" => {
            let n = json_usize(val)?;
            if n < 4096 {
                return Err(FcpError::Config(
                    "moltbook.max_response_bytes must be at least 4096".into(),
                ));
            }
            set_nested_integer(doc, &["moltbook"], "max_response_bytes", n as u64);
        }
        "vision.enabled" => {
            set_nested_bool(doc, &["vision"], "enabled", json_bool(val)?);
        }
        "audio.enabled" => {
            set_nested_bool(doc, &["audio"], "enabled", json_bool(val)?);
        }
        "audio.max_duration_secs" => {
            let n = json_u64(val)?;
            if n == 0 || n > 600 {
                return Err(FcpError::Config(
                    "audio.max_duration_secs must be 1..600".into(),
                ));
            }
            set_nested_integer(doc, &["audio"], "max_duration_secs", n);
        }
        "discord.enabled" => {
            set_nested_bool(doc, &["discord"], "enabled", json_bool(val)?);
        }
        "discord.channel_name" => {
            let s = json_string(val)?.trim().to_string();
            if s.is_empty() {
                set_nested_remove(doc, &["discord"], "channel_name");
            } else {
                set_nested_string(doc, &["discord"], "channel_name", s);
            }
        }
        "discord.channel_id" => {
            let s = json_string(val)?.trim().to_string();
            if s.is_empty() {
                set_nested_remove(doc, &["discord"], "channel_id");
            } else {
                let n: u64 = s
                    .parse()
                    .map_err(|_| FcpError::Config("discord.channel_id must be a number".into()))?;
                set_nested_integer(doc, &["discord"], "channel_id", n);
            }
        }
        other => {
            return Err(FcpError::Config(format!("unsupported tool config key: {other}")));
        }
    }
    Ok(())
}

fn read_bool_key(doc: &toml::Table, key: &str) -> Option<bool> {
    doc.get(key).and_then(|v| v.as_bool())
}

fn sync_api_profiles_enabled(doc: &mut toml::Table, profile_ids: &[&str], enabled: bool) {
    let apis_entry = doc
        .entry("apis".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(apis_table) = apis_entry.as_table_mut() else {
        return;
    };
    for id in profile_ids {
        let entry = apis_table
            .entry((*id).to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let Some(t) = entry.as_table_mut() {
            t.insert("enabled".to_string(), toml::Value::Boolean(enabled));
        }
    }
}

fn set_nested_bool(doc: &mut toml::Table, section: &[&str], key: &str, value: bool) {
    let Some(table) = nested_table_mut(doc, section) else {
        return;
    };
    table.insert(key.to_string(), toml::Value::Boolean(value));
}

fn set_nested_string(doc: &mut toml::Table, section: &[&str], key: &str, value: String) {
    let Some(table) = nested_table_mut(doc, section) else {
        return;
    };
    table.insert(key.to_string(), toml::Value::String(value));
}

fn set_nested_integer(doc: &mut toml::Table, section: &[&str], key: &str, value: u64) {
    let Some(table) = nested_table_mut(doc, section) else {
        return;
    };
    table.insert(key.to_string(), toml::Value::Integer(value as i64));
}

fn set_nested_remove(doc: &mut toml::Table, section: &[&str], key: &str) {
    let Some(table) = nested_table_mut(doc, section) else {
        return;
    };
    table.remove(key);
}

fn nested_table_mut<'a>(doc: &'a mut toml::Table, section: &[&str]) -> Option<&'a mut toml::Table> {
    let mut current = doc;
    for (i, part) in section.iter().enumerate() {
        if i == section.len() - 1 {
            let entry = current
                .entry((*part).to_string())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            return entry.as_table_mut();
        }
        let entry = current
            .entry((*part).to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        current = entry.as_table_mut()?;
    }
    None
}

fn json_usize(v: &JsonValue) -> Result<usize> {
    match v {
        JsonValue::Number(n) => n
            .as_u64()
            .map(|u| u as usize)
            .ok_or_else(|| FcpError::Config("expected unsigned integer".into())),
        _ => Err(FcpError::Config("expected number".into())),
    }
}

fn json_u64(v: &JsonValue) -> Result<u64> {
    match v {
        JsonValue::Number(n) => n
            .as_u64()
            .ok_or_else(|| FcpError::Config("expected unsigned integer".into())),
        _ => Err(FcpError::Config("expected number".into())),
    }
}

fn json_f64(v: &JsonValue) -> Result<f64> {
    match v {
        JsonValue::Number(n) => n
            .as_f64()
            .ok_or_else(|| FcpError::Config("expected float".into())),
        _ => Err(FcpError::Config("expected number".into())),
    }
}

fn json_bool(v: &JsonValue) -> Result<bool> {
    match v {
        JsonValue::Bool(b) => Ok(*b),
        _ => Err(FcpError::Config("expected boolean".into())),
    }
}

fn json_string(v: &JsonValue) -> Result<String> {
    match v {
        JsonValue::String(s) => Ok(s.clone()),
        _ => Err(FcpError::Config("expected string".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn merge_weather_toggle_syncs_api_profiles() {
        let dir = TempDir::new().expect("tempdir");
        let fcp = dir.path().join(".fcp");
        std::fs::create_dir_all(&fcp).expect("mkdir");
        let cfg_path = fcp.join("config.toml");
        let mut f = std::fs::File::create(&cfg_path).expect("create");
        writeln!(
            f,
            r#"
workspace = "t"
weather_enabled = true

[apis.open_meteo_geocode]
enabled = true
base_url = "https://example.com"
"#
        )
        .expect("write");

        let config = AppConfig::default();
        let payload = ToolsUpdatePayload {
            family_id: "weather".into(),
            values: BTreeMap::from([(
                "weather_enabled".to_string(),
                JsonValue::from(false),
            )]),
        };
        merge_tools_into_toml(dir.path(), &config, &payload)
            .await
            .expect("merge");
        let raw = tokio::fs::read_to_string(&cfg_path).await.expect("read");
        assert!(raw.contains("weather_enabled = false"));
        assert!(raw.contains("enabled = false"));
    }
}
