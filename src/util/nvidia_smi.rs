//! Optional `nvidia-smi` subprocess for host GPU metrics (Linux + NVIDIA driver).
//!
//! Probes are off under `cargo test` and CI unless `FCP_FORCE_NVIDIA_SMI=1`.
//! Set `FCP_SKIP_NVIDIA_SMI=1` to disable on a normal workstation.

use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn ci_like_environment() -> bool {
    ["CI", "GITHUB_ACTIONS", "JENKINS_URL", "GITLAB_CI", "BUILD_BUILDID"]
        .into_iter()
        .any(|key| std::env::var(key).is_ok())
}

/// Returns true if Eris may run the host `nvidia-smi` executable for health diagnostics.
///
/// Disabled when:
/// - `FCP_SKIP_NVIDIA_SMI=1`, or
/// - a common CI environment variable is set, or
/// - the crate is built with `cfg!(test)`,
///
/// unless `FCP_FORCE_NVIDIA_SMI=1` is set.
pub fn nvidia_smi_subprocess_allowed() -> bool {
    if std::env::var("FCP_FORCE_NVIDIA_SMI").as_deref() == Ok("1") {
        return true;
    }
    if std::env::var("FCP_SKIP_NVIDIA_SMI").as_deref() == Ok("1") {
        return false;
    }
    if ci_like_environment() {
        return false;
    }
    if cfg!(test) {
        return false;
    }
    true
}

/// True if `nvidia-smi` resolves on PATH and `--version` exits successfully.
pub fn nvidia_smi_binary_reachable() -> bool {
    match Command::new("nvidia-smi")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

fn split_csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(cur.trim().trim_matches('"').to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    fields.push(cur.trim().trim_matches('"').to_string());
    fields
}

fn parse_query_token(s: &str) -> Value {
    let t = s.trim();
    let t = t.trim_matches(|c| c == '[' || c == ']');
    let t = t.trim();
    if t.eq_ignore_ascii_case("n/a") || t.is_empty() {
        return Value::Null;
    }
    if let Ok(i) = t.parse::<u64>() {
        return json!(i);
    }
    if let Ok(i) = t.parse::<i64>() {
        return json!(i);
    }
    if let Ok(f) = t.parse::<f64>() {
        return json!(f);
    }
    json!(t)
}

fn parse_gpu_csv(stdout: &str) -> Result<Vec<Value>, String> {
    let mut gpus = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols = split_csv_fields(line);
        if cols.len() < 6 {
            return Err(format!(
                "expected at least 6 columns per GPU line, got {}: {:?}",
                cols.len(),
                line.chars().take(120).collect::<String>()
            ));
        }
        let index_raw = &cols[0];
        let name = cols[1].clone();
        let mem_used = parse_query_token(&cols[2]);
        let mem_total = parse_query_token(&cols[3]);
        let util = parse_query_token(&cols[4]);
        let temp_c = parse_query_token(&cols[5]);
        let index = index_raw
            .trim()
            .parse::<u32>()
            .map_err(|e| format!("gpu index: {e} ({index_raw:?})"))?;
        gpus.push(json!({
            "index": index,
            "name": name,
            "memory_used_mib": mem_used,
            "memory_total_mib": mem_total,
            "utilization_gpu_pct": util,
            "temperature_c": temp_c,
        }));
    }
    Ok(gpus)
}

/// JSON blob for `system:health` under `gpu.nvidia_smi`.
pub fn run_nvidia_smi_health_json() -> Value {
    if !nvidia_smi_subprocess_allowed() {
        return json!({
            "available": false,
            "skipped": "nvidia-smi probe disabled under test/CI (FCP_FORCE_NVIDIA_SMI=1 to enable)",
        });
    }
    if !nvidia_smi_binary_reachable() {
        return json!({
            "available": false,
            "reason": "not_on_path",
        });
    }

    match Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.used,memory.total,utilization.gpu,temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code();
            if !output.status.success() {
                return json!({
                    "available": false,
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                    "reason": "nvidia_smi_failed",
                });
            }
            match parse_gpu_csv(&stdout) {
                Ok(gpus) => json!({
                    "available": true,
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                    "gpus": gpus,
                }),
                Err(parse_error) => json!({
                    "available": false,
                    "exit_code": exit_code,
                    "stdout": stdout,
                    "stderr": stderr,
                    "reason": "parse_error",
                    "parse_error": parse_error,
                }),
            }
        }
        Err(e) => json!({
            "available": false,
            "error": e.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_disabled_in_test_binary_unless_forced() {
        if std::env::var("FCP_FORCE_NVIDIA_SMI").as_deref() == Ok("1") {
            assert!(nvidia_smi_subprocess_allowed());
            return;
        }
        assert!(
            !nvidia_smi_subprocess_allowed(),
            "unit tests must not shell out to nvidia-smi by default"
        );
    }

    #[test]
    fn parse_gpu_csv_simple_line() {
        let stdout = "0, NVIDIA GeForce RTX 3060, 100, 12288, 0, 45\n";
        let gpus = parse_gpu_csv(stdout).expect("parse");
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0]["index"], 0);
        assert_eq!(gpus[0]["name"], "NVIDIA GeForce RTX 3060");
    }

    #[test]
    fn parse_gpu_csv_quoted_name_with_comma() {
        let stdout = r#"0, "Foo, Bar GPU", 1, 8192, 12, [N/A]"#;
        let gpus = parse_gpu_csv(stdout).expect("parse");
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0]["name"], "Foo, Bar GPU");
        assert!(gpus[0]["temperature_c"].is_null());
    }
}
