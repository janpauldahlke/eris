use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use sysinfo::{Disks, System};

use crate::executive::error::Result;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct SystemHealthArgs {}

pub struct SystemHealthTool;

#[async_trait]
impl Tool for SystemHealthTool {
    fn name(&self) -> &'static str {
        "system:health"
    }

    fn description(&self) -> &'static str {
        "Returns CPU, RAM, disk, OS metadata, and ollama ps status as JSON."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(SystemHealthArgs)
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        // Gather metrics in a blocking task to avoid stalling the async runtime.
        let health = tokio::task::spawn_blocking(|| {
            let mut system = System::new_all();
            system.refresh_all();

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

            let ollama = match std::process::Command::new("ollama").arg("ps").output() {
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
            };

            let load = System::load_average();
            json!({
                "system": {
                    "os_name": System::name(),
                    "os_version": System::os_version(),
                    "kernel_version": System::kernel_version(),
                    "host_name": System::host_name(),
                    "uptime_secs": System::uptime(),
                    "cpu_usage_pct": system.global_cpu_usage(),
                    "logical_cpus": system.cpus().len(),
                    "load_avg_one": load.one,
                    "load_avg_five": load.five,
                    "load_avg_fifteen": load.fifteen,
                    "memory_total_bytes": system.total_memory(),
                    "memory_used_bytes": system.used_memory(),
                    "memory_available_bytes": system.available_memory(),
                    "swap_total_bytes": system.total_swap(),
                    "swap_used_bytes": system.used_swap(),
                },
                "disks": disk_entries,
                "ollama_ps": ollama
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
        let tool = SystemHealthTool;
        let args = serde_json::json!({});

        let result = tool.execute(args).await.expect("system health tool should succeed");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("result should be valid JSON");

        assert!(parsed.get("system").is_some());
        assert!(parsed.get("disks").is_some());
        assert!(parsed.get("ollama_ps").is_some());
    }
}
