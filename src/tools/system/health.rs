use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use sysinfo::{Disks, System};

use crate::config::{AppConfig, LlmBackend};
use crate::executive::error::Result;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct SystemHealthArgs {}

pub struct SystemHealthTool {
    pub config: Arc<AppConfig>,
}

const REPORT_HINT_OLLAMA: &str = "When answering the user, always cover in order: (1) `llm_backend` and `fcp`: Ollama URL and chat + embed models; (2) `ollama.cli_ps`: whether the CLI ran and summarize stdout or error; (3) `cpu.usage_pct` and load averages; (4) `memory` used vs total and `used_pct`. If `gpu.nvidia_smi.available` is true, summarize per-GPU memory, utilization, and temperature from `gpus`; if `available` is false and `reason` is `not_on_path`, omit GPU detail; if `skipped` is present, omit GPU detail. Optionally mention `host` and `disks` if relevant.";

const REPORT_HINT_LLAMACPP: &str = "When answering the user, cover: (1) `llm_backend` and `fcp` (chat/embed servers and GGUF model paths); (2) `llama_cpp_health` server statuses; (3) `cpu.usage_pct` and load averages; (4) `memory` used vs total. If `gpu.nvidia_smi.available` is true, summarize GPU info from `gpus`; if skipped, omit GPU detail. Optionally mention `host` and `disks` if relevant.";

async fn probe_llama_health(base_url: String) -> String {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("unreachable: {e}"),
    };
    let health_url = format!("{}/health", base_url.trim_end_matches('/'));
    let resp = match tokio::time::timeout(Duration::from_secs(4), client.get(health_url).send()).await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return format!("unreachable: {e}"),
        Err(_) => return "unreachable: timeout".to_string(),
    };
    if !resp.status().is_success() {
        return format!("unreachable: HTTP {}", resp.status());
    }
    let body = match resp.text().await {
        Ok(t) => t.to_ascii_lowercase(),
        Err(e) => return format!("unreachable: {e}"),
    };
    if body.contains("loading") {
        "loading model".to_string()
    } else {
        "ok".to_string()
    }
}

#[async_trait]
impl Tool for SystemHealthTool {
    fn name(&self) -> &'static str {
        "system:health"
    }

    fn description(&self) -> &'static str {
        "Structured host diagnostics JSON with stable sections: `report_hint` (how to summarize), `llm_backend`, `fcp` (Ollama or llama-server targets), optional `llama_cpp_health` when using llama.cpp, `cpu`, `memory`, `ollama` (`ollama ps` when Ollama is the LLM backend), `gpu.nvidia_smi` (optional), plus `host` and `disks`. Follow `report_hint`."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SystemHealthArgs)
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let cfg = self.config.clone();
        let report_hint = if cfg.is_llamacpp() {
            REPORT_HINT_LLAMACPP
        } else {
            REPORT_HINT_OLLAMA
        };

        let fcp_section = match cfg.llm_backend {
            LlmBackend::Ollama => json!({
                "ollama_host": cfg.ollama_host.as_str(),
                "chat_model": cfg.model_name.as_str(),
                "embed_model": cfg.embed_model_name.as_str(),
            }),
            LlmBackend::LlamaCpp => {
                if let Some(lc) = cfg.llama_cpp.as_ref() {
                    json!({
                        "chat_server": lc.chat_server_url.as_str(),
                        "chat_model": lc.chat_model_path.display().to_string(),
                        "embed_server": lc.embed_server_url.as_str(),
                        "embed_model": lc.embed_model_path.display().to_string(),
                    })
                } else {
                    json!({
                        "error": "llm_backend is LlamaCpp but llama_cpp section is missing",
                    })
                }
            }
        };

        let llama_health = if cfg.is_llamacpp() {
            if let Some(lc) = cfg.llama_cpp.as_ref() {
                let chat = probe_llama_health(lc.chat_server_url.clone()).await;
                let embed = probe_llama_health(lc.embed_server_url.clone()).await;
                Some(json!({
                    "chat_server_status": chat,
                    "embed_server_status": embed,
                }))
            } else {
                None
            }
        } else {
            None
        };

        let skip_ollama_ps = !cfg.is_ollama();
        let llm_backend_json = serde_json::to_value(&cfg.llm_backend)
            .unwrap_or_else(|_| json!(cfg.llm_backend.to_string()));
        let fcp_moved = fcp_section;
        let llama_moved = llama_health;

        let health = tokio::task::spawn_blocking(move || {
            let mut system = System::new_all();
            system.refresh_all();

            let total_mem = system.total_memory();
            let used_mem = system.used_memory();
            let used_pct = if total_mem > 0 {
                (used_mem as f64 / total_mem as f64) * 100.0
            } else {
                0.0
            };

            let disks = Disks::new_with_refreshed_list();
            let disk_entries = disks
                .list()
                .iter()
                .map(|disk| {
                    let total = disk.total_space();
                    let available = disk.available_space();
                    let used = total.saturating_sub(available);
                    json!({
                        "name": disk.name().to_string_lossy().to_string(),
                        "mount_point": disk.mount_point().display().to_string(),
                        "total_bytes": total,
                        "used_bytes": used,
                        "available_bytes": available,
                    })
                })
                .collect::<Vec<Value>>();

            let ollama_cli = if skip_ollama_ps {
                json!({
                    "skipped": "llama.cpp backend active",
                })
            } else if crate::util::ollama_host_cli::host_ollama_cli_subprocess_allowed() {
                match std::process::Command::new("ollama").arg("ps").output() {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        json!({
                            "available": true,
                            "exit_code": output.status.code(),
                            "stdout": stdout,
                            "stderr": stderr,
                        })
                    }
                    Err(e) => {
                        json!({
                            "available": false,
                            "error": e.to_string(),
                        })
                    }
                }
            } else {
                json!({
                    "available": false,
                    "skipped": "host ollama CLI disabled under test/CI (FCP_FORCE_HOST_OLLAMA_CLI=1 to enable)",
                })
            };

            let gpu = json!({
                "nvidia_smi": crate::util::nvidia_smi::run_nvidia_smi_health_json(),
            });

            let load = System::load_average();

            let mut map = serde_json::Map::new();
            map.insert("report_hint".into(), json!(report_hint));
            map.insert("llm_backend".into(), llm_backend_json);
            map.insert("fcp".into(), fcp_moved);
            map.insert(
                "cpu".into(),
                json!({
                    "usage_pct": system.global_cpu_usage(),
                    "logical_cpus": system.cpus().len(),
                    "load_avg_one": load.one,
                    "load_avg_five": load.five,
                    "load_avg_fifteen": load.fifteen,
                }),
            );
            map.insert(
                "memory".into(),
                json!({
                    "total_bytes": total_mem,
                    "used_bytes": used_mem,
                    "available_bytes": system.available_memory(),
                    "used_pct": used_pct,
                    "swap_total_bytes": system.total_swap(),
                    "swap_used_bytes": system.used_swap(),
                }),
            );
            map.insert(
                "ollama".into(),
                json!({
                    "cli_ps": ollama_cli,
                }),
            );
            map.insert("gpu".into(), gpu);
            map.insert(
                "host".into(),
                json!({
                    "os_name": System::name(),
                    "os_version": System::os_version(),
                    "kernel_version": System::kernel_version(),
                    "host_name": System::host_name(),
                    "uptime_secs": System::uptime(),
                }),
            );
            map.insert("disks".into(), json!(disk_entries));

            if let Some(h) = llama_moved {
                map.insert("llama_cpp_health".into(), h);
            }

            serde_json::Value::Object(map).to_string()
        })
        .await
        .map_err(|e| crate::executive::error::FcpError::EngineFault(e.to_string()))?;

        Ok(health)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_system_health_execution() {
        let tool = SystemHealthTool {
            config: Arc::new(AppConfig::default()),
        };
        let args = serde_json::json!({});

        let result = tool
            .execute(args)
            .await
            .expect("system health tool should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result should be valid JSON");

        assert!(parsed.get("report_hint").is_some());
        assert_eq!(parsed.get("llm_backend"), Some(&json!("Ollama")));
        assert!(parsed.get("fcp").is_some());
        assert!(
            parsed
                .get("fcp")
                .and_then(|x| x.get("ollama_host"))
                .is_some()
        );
        assert!(
            parsed
                .get("fcp")
                .and_then(|x| x.get("chat_model"))
                .is_some()
        );
        assert!(parsed.get("cpu").and_then(|x| x.get("usage_pct")).is_some());
        assert!(
            parsed
                .get("memory")
                .and_then(|x| x.get("used_pct"))
                .is_some()
        );
        assert!(parsed.get("ollama").is_some());
        assert!(parsed.get("llama_cpp_health").is_none());
        let gpu = parsed.get("gpu").expect("gpu section");
        let nsmi = gpu.get("nvidia_smi").expect("gpu.nvidia_smi");
        if std::env::var("FCP_FORCE_NVIDIA_SMI").as_deref() == Ok("1") {
            assert!(
                nsmi.get("skipped").is_none(),
                "when FCP_FORCE_NVIDIA_SMI=1 the probe is not skipped"
            );
            assert!(nsmi.get("available").is_some());
        } else {
            assert!(
                nsmi.get("skipped").is_some(),
                "nvidia_smi skipped by default in unit tests"
            );
        }
        assert!(parsed.get("host").is_some());
        assert!(parsed.get("disks").is_some());
    }

    #[tokio::test]
    async fn health_output_ollama_backend() {
        let tool = SystemHealthTool {
            config: Arc::new(AppConfig::default()),
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&tool.execute(json!({})).await.expect("health")).expect("json");
        assert_eq!(parsed.get("llm_backend"), Some(&json!("Ollama")));
        assert!(parsed.get("fcp").and_then(|f| f.get("ollama_host")).is_some());
        assert!(parsed.get("llama_cpp_health").is_none());
        let ollama = parsed.get("ollama").expect("ollama");
        assert!(!ollama
            .get("cli_ps")
            .and_then(|c| c.get("skipped"))
            .and_then(|s| s.as_str())
            .is_some_and(|t| t.contains("llama.cpp")));
    }

    #[tokio::test]
    async fn health_output_llamacpp_backend() {
        use crate::config::LlamaCppConfig;
        use std::path::PathBuf;

        let mut cfg = AppConfig::default();
        cfg.llm_backend = LlmBackend::LlamaCpp;
        cfg.llama_cpp = Some(LlamaCppConfig {
            home: PathBuf::from("/tmp"),
            chat_server_url: "http://127.0.0.1:9".into(),
            embed_server_url: "http://127.0.0.1:9".into(),
            chat_model_path: PathBuf::from("/models/chat.gguf"),
            embed_model_path: PathBuf::from("/models/embed.gguf"),
            ctx_size: 4096,
            n_gpu_layers: 0,
            ready_timeout_secs: 1,
        });
        let tool = SystemHealthTool {
            config: Arc::new(cfg),
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&tool.execute(json!({})).await.expect("health")).expect("json");
        assert_eq!(parsed.get("llm_backend"), Some(&json!("LlamaCpp")));
        let fcp = parsed.get("fcp").expect("fcp");
        assert!(fcp.get("chat_server").is_some());
        assert!(fcp.get("embed_server").is_some());
        let lh = parsed.get("llama_cpp_health").expect("llama health");
        assert!(lh.get("chat_server_status").is_some());
        assert!(lh.get("embed_server_status").is_some());
        let cli = parsed
            .pointer("/ollama/cli_ps/skipped")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(cli.contains("llama.cpp"));
    }
}
