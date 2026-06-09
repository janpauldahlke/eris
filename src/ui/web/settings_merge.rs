//! Partial merge of operator-editable keys into `.fcp/config.toml` from the web console.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};
use crate::vault_layout;

/// Editable settings payload from `PUT /api/console/settings`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SettingsUpdatePayload {
    #[serde(default)]
    pub values: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsFieldSchema {
    pub key: String,
    pub value: JsonValue,
    pub label: String,
    pub description: String,
    pub impact: String,
    pub restart_required: bool,
    pub editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warn_above: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsSchemaResponse {
    pub fields: Vec<SettingsFieldSchema>,
    pub num_ctx_max: usize,
    pub num_ctx_warn_above: usize,
}

pub fn build_settings_schema(config: &AppConfig) -> SettingsSchemaResponse {
    let num_ctx_max = config.web_ui.settings.num_ctx_max.max(4096);
    let num_ctx_warn_above = config.web_ui.settings.num_ctx_warn_above;
    SettingsSchemaResponse {
        num_ctx_max,
        num_ctx_warn_above,
        fields: vec![
            field_usize(
                "num_ctx",
                config.num_ctx,
                "Context window",
                "Tokens reserved for the chat stack and LLM KV cache. Higher values use more VRAM and can OOM.",
                "Affects condensation thresholds, vault read budgets, and llama-server --ctx-size on restart.",
                true,
                Some(num_ctx_max),
                Some(num_ctx_warn_above),
            ),
            field_u64(
                "generation_timeout_secs",
                config.generation_timeout_secs,
                "Generation timeout",
                "Max seconds to wait for one LLM completion.",
                "Turns fail with a timeout error if the model exceeds this wall clock.",
                true,
            ),
            field_u8(
                "max_tool_rounds",
                config.max_tool_rounds,
                "Max tool rounds",
                "Tool-call hops allowed per user turn before cap recovery.",
                "Higher values allow longer agent chains but increase latency and token use.",
                true,
            ),
            field_u8(
                "max_recovery_attempts",
                config.max_recovery_attempts,
                "Max recovery attempts",
                "JSON/schema recovery retries per turn.",
                "More retries tolerate flaky model output but extend turn duration.",
                true,
            ),
            field_bool(
                "memory_prefetch_enabled",
                config.memory_prefetch_enabled,
                "Memory prefetch",
                "Embed the user message and inject Qdrant hits before each LLM step.",
                "Changes turn-start context without tool calls.",
                true,
            ),
            field_usize(
                "memory_prefetch_top_k",
                config.memory_prefetch_top_k as usize,
                "Prefetch top K",
                "Number of Qdrant hits to inject at turn start.",
                "More hits add context but consume prompt tokens.",
                true,
                None,
                None,
            ),
            field_f64(
                "memory_prefetch_min_score",
                f64::from(config.memory_prefetch_min_score),
                "Prefetch min score",
                "Minimum similarity score for prefetch hits (0–1).",
                "Higher values reduce noise but may miss relevant memory.",
                true,
            ),
            field_string_readonly(
                "llm_backend",
                config.llm_backend.to_string(),
                "LLM backend",
                "Engine selected at session start (Ollama or LlamaCpp).",
                "Cannot be changed while Eris is running.",
            ),
            field_string_readonly(
                "model_name",
                config.model_name.clone(),
                "Model name",
                "Chat model id for the active backend.",
                "Change in config.toml before starting a new session.",
            ),
            field_string_readonly(
                "workspace",
                config.workspace.clone(),
                "Workspace",
                "Logical vault id (Qdrant collection suffix).",
                "Fixed for this running session.",
            ),
            field_usize_readonly("num_ctx_current", config.num_ctx, "Current num_ctx"),
            field_string_readonly(
                "qdrant_url",
                config.qdrant_url.clone(),
                "Qdrant URL",
                "Semantic memory endpoint.",
                "Restart required after change.",
            ),
        ],
    }
}

fn field_usize(
    key: &str,
    value: usize,
    label: &str,
    description: &str,
    impact: &str,
    editable: bool,
    max_value: Option<usize>,
    warn_above: Option<usize>,
) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value,
        warn_above,
    }
}

fn field_u64(key: &str, value: u64, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_u8(key: &str, value: u8, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_bool(key: &str, value: bool, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_f64(key: &str, value: f64, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: serde_json::Number::from_f64(value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::from(0)),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_string_readonly(key: &str, value: String, label: &str, description: &str, impact: &str) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::String(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: false,
        editable: false,
        max_value: None,
        warn_above: None,
    }
}

fn field_usize_readonly(key: &str, value: usize, label: &str) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: "Effective value for this session.".to_string(),
        impact: "Read-only.".to_string(),
        restart_required: false,
        editable: false,
        max_value: None,
        warn_above: None,
    }
}

/// Apply allowed keys from the payload into the on-disk TOML config.
pub async fn merge_settings_into_toml(
    vault_root: &Path,
    config: &AppConfig,
    payload: &SettingsUpdatePayload,
) -> Result<()> {
    let path = vault_layout::config_toml(vault_root);
    let raw = tokio::fs::read_to_string(&path).await.map_err(FcpError::Io)?;
    let mut doc: toml::Table = toml::from_str(&raw)
        .map_err(|e| FcpError::Config(format!("parse config.toml: {e}")))?;

    let num_ctx_max = config.web_ui.settings.num_ctx_max.max(4096);

    for (key, val) in &payload.values {
        match key.as_str() {
            "num_ctx" => {
                let n = json_usize(val)?;
                if n < 1024 {
                    return Err(FcpError::Config(
                        "num_ctx must be at least 1024".into(),
                    ));
                }
                if n > num_ctx_max {
                    return Err(FcpError::Config(format!(
                        "num_ctx {n} exceeds web UI cap {num_ctx_max}"
                    )));
                }
                doc.insert("num_ctx".into(), toml::Value::Integer(n as i64));
            }
            "generation_timeout_secs" => {
                let n = json_u64(val)?;
                if n == 0 {
                    return Err(FcpError::Config(
                        "generation_timeout_secs must be positive".into(),
                    ));
                }
                doc.insert(
                    "generation_timeout_secs".into(),
                    toml::Value::Integer(n as i64),
                );
            }
            "max_tool_rounds" => {
                let n = json_u64(val)?;
                if n == 0 || n > 50 {
                    return Err(FcpError::Config(
                        "max_tool_rounds must be 1..50".into(),
                    ));
                }
                doc.insert("max_tool_rounds".into(), toml::Value::Integer(n as i64));
            }
            "max_recovery_attempts" => {
                let n = json_u64(val)?;
                if n > 20 {
                    return Err(FcpError::Config(
                        "max_recovery_attempts must be <= 20".into(),
                    ));
                }
                doc.insert(
                    "max_recovery_attempts".into(),
                    toml::Value::Integer(n as i64),
                );
            }
            "memory_prefetch_enabled" => {
                doc.insert(
                    "memory_prefetch_enabled".into(),
                    toml::Value::Boolean(json_bool(val)?),
                );
            }
            "memory_prefetch_top_k" => {
                let n = json_usize(val)?;
                if n == 0 || n > 25 {
                    return Err(FcpError::Config(
                        "memory_prefetch_top_k must be 1..25".into(),
                    ));
                }
                doc.insert(
                    "memory_prefetch_top_k".into(),
                    toml::Value::Integer(n as i64),
                );
            }
            "memory_prefetch_min_score" => {
                let f = json_f64(val)?;
                if !(0.0..=1.0).contains(&f) {
                    return Err(FcpError::Config(
                        "memory_prefetch_min_score must be 0..1".into(),
                    ));
                }
                doc.insert(
                    "memory_prefetch_min_score".into(),
                    toml::Value::Float(f),
                );
            }
            other => {
                tracing::warn!(
                    key = %other,
                    "web settings: ignored unknown editable key"
                );
            }
        }
    }

    let out = toml::to_string_pretty(&doc)
        .map_err(|e| FcpError::Config(format!("serialize config.toml: {e}")))?;
    tokio::fs::write(&path, out.as_bytes())
        .await
        .map_err(FcpError::Io)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn merge_num_ctx_clamped_by_web_ui_max() {
        let dir = TempDir::new().expect("tempdir");
        let fcp = dir.path().join(".fcp");
        std::fs::create_dir_all(&fcp).expect("mkdir");
        let cfg_path = fcp.join("config.toml");
        let mut f = std::fs::File::create(&cfg_path).expect("create");
        writeln!(f, "workspace = \"t\"\nnum_ctx = 8192").expect("write");

        let mut config = AppConfig::default();
        config.web_ui.settings.num_ctx_max = 16384;
        config.workspace = "t".into();

        let payload = SettingsUpdatePayload {
            values: BTreeMap::from([(
                "num_ctx".to_string(),
                JsonValue::from(32768_usize),
            )]),
        };
        let err = merge_settings_into_toml(dir.path(), &config, &payload)
            .await
            .expect_err("over max");
        assert!(err.to_string().contains("exceeds"));

        let payload_ok = SettingsUpdatePayload {
            values: BTreeMap::from([("num_ctx".to_string(), JsonValue::from(12288_usize))]),
        };
        merge_settings_into_toml(dir.path(), &config, &payload_ok)
            .await
            .expect("ok");
        let raw = tokio::fs::read_to_string(&cfg_path).await.expect("read");
        assert!(raw.contains("num_ctx = 12288"));
    }
}
