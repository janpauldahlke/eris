//! Whether this process may shell out to the host `ollama` binary (`ps`, `stop`, etc.).
//!
//! `cargo test` and most CI runners should **not** invoke the real CLI: on macOS that can
//! surface the Ollama GUI, and headless agents lack Ollama entirely.

fn ci_like_environment() -> bool {
    [
        "CI",
        "GITHUB_ACTIONS",
        "JENKINS_URL",
        "GITLAB_CI",
        "BUILD_BUILDID",
    ]
    .into_iter()
    .any(|key| std::env::var(key).is_ok())
}

/// Returns true if Eris may run the host `ollama` executable (e.g. `ollama ps`, `ollama stop`).
///
/// Disabled when:
/// - a common CI environment variable is set (`CI`, `GITHUB_ACTIONS`, `JENKINS_URL`, `GITLAB_CI`,
///   or Azure Pipelines `BUILD_BUILDID`), or
/// - the crate is built with `cfg(test)` (`cargo test` for this crate’s unit tests),
///
/// unless `FCP_FORCE_HOST_OLLAMA_CLI=1` is set (local override).
///
/// Set `FCP_SKIP_HOST_OLLAMA_CLI=1` to disable probes on a normal workstation (e.g. avoid GUI).
pub fn host_ollama_cli_subprocess_allowed() -> bool {
    if std::env::var("FCP_FORCE_HOST_OLLAMA_CLI").as_deref() == Ok("1") {
        return true;
    }
    if std::env::var("FCP_SKIP_HOST_OLLAMA_CLI").as_deref() == Ok("1") {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_cli_disabled_in_test_binary_unless_forced() {
        if std::env::var("FCP_FORCE_HOST_OLLAMA_CLI").as_deref() == Ok("1") {
            assert!(host_ollama_cli_subprocess_allowed());
            return;
        }
        assert!(
            !host_ollama_cli_subprocess_allowed(),
            "unit tests must not shell out to ollama by default"
        );
    }
}
