use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use sysinfo::{Disks, System};

use crate::config::AppConfig;
use crate::executive::error::Result;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct SystemHealthArgs {}

pub struct SystemHealthTool {
    pub config: Arc<AppConfig>,
}

const REPORT_HINT: &str = "When answering the user, always cover in order: (1) `fcp`: Ollama URL and chat + embed models; (2) `ollama.cli_ps`: whether the CLI ran and summarize stdout or error; (3) `cpu.usage_pct` and load averages; (4) `memory` used vs total and `used_pct`. If `gpu.nvidia_smi.available` is true, summarize per-GPU memory, utilization, and temperature from `gpus`; if `available` is false and `reason` is `not_on_path`, omit GPU detail; if `skipped` is present, omit GPU detail. Optionally mention `host` and `disks` if relevant.";

#[async_trait]
impl Tool for SystemHealthTool {
    fn name(&self) -> &'static str {
        "system:health"
    }

    fn description(&self) -> &'static str {
        "Structured host diagnostics JSON with stable sections: `report_hint` (how to summarize), `fcp` (configured Ollama host and models), `cpu`, `memory`, `ollama` (`ollama ps`), `gpu.nvidia_smi` (optional NVIDIA GPUs via `nvidia-smi` when on PATH), plus `host` and `disks`. Follow `report_hint` so answers consistently mention Ollama, models, CPU, and RAM, and GPU when available."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SystemHealthArgs)
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let cfg = self.config.clone();
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

            let ollama_cli = if crate::util::ollama_host_cli::host_ollama_cli_subprocess_allowed() {
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
            // Key order preserved (serde_json preserve_order): snippet truncation keeps the hint and core metrics first.
            json!({
                "report_hint": REPORT_HINT,
                "fcp": {
                    "ollama_host": cfg.ollama_host.as_str(),
                    "chat_model": cfg.model_name.as_str(),
                    "embed_model": cfg.embed_model_name.as_str(),
                },
                "cpu": {
                    "usage_pct": system.global_cpu_usage(),
                    "logical_cpus": system.cpus().len(),
                    "load_avg_one": load.one,
                    "load_avg_five": load.five,
                    "load_avg_fifteen": load.fifteen,
                },
                "memory": {
                    "total_bytes": total_mem,
                    "used_bytes": used_mem,
                    "available_bytes": system.available_memory(),
                    "used_pct": used_pct,
                    "swap_total_bytes": system.total_swap(),
                    "swap_used_bytes": system.used_swap(),
                },
                "ollama": {
                    "cli_ps": ollama_cli,
                },
                "gpu": gpu,
                "host": {
                    "os_name": System::name(),
                    "os_version": System::os_version(),
                    "kernel_version": System::kernel_version(),
                    "host_name": System::host_name(),
                    "uptime_secs": System::uptime(),
                },
                "disks": disk_entries,
            })
            .to_string()
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
        assert!(parsed.get("fcp").is_some());
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
}
