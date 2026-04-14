//! Blocking `inquire` prompts — only run when stdin is a TTY (caller checks).

use std::path::Path;

use inquire::{Confirm, Text};

use crate::executive::cli::Cli;
use crate::executive::error::{FcpError, Result};

use super::report::WelderReport;
use super::IgnitionWorkspaceHint;

pub(crate) fn sanitize_workspace_id(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "default".into()
    } else {
        s.chars().take(64).collect()
    }
}

fn folder_hint(workspace_root: &Path) -> String {
    workspace_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("vault")
        .to_string()
}

fn exe_looks_like_downloads_or_tmp() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let lossy = exe.to_string_lossy();
    let lower = lossy.to_lowercase();
    lower.contains("downloads")
        || lower.contains("/tmp/")
        || lower.contains("\\temp\\")
}

fn print_binary_layout_tip() {
    eprintln!();
    eprintln!("Tip: keep the `eris` binary outside Downloads.");
    eprintln!("  mkdir -p \"$HOME/eris/bin\" && mv \"$HOME/Downloads/eris\" \"$HOME/eris/bin/eris\" && chmod +x \"$HOME/eris/bin/eris\"");
    eprintln!("  echo 'export PATH=\"$HOME/eris/bin:$PATH\"' >> ~/.zshrc   # or ~/.bashrc on Linux");
    eprintln!("Your vault is separate: create a folder, `cd` into it, then run `eris chat`.");
    eprintln!();
}

/// Run interactive first-run prompts; must be called from `spawn_blocking` or before blocking the runtime.
pub fn run_interactive_sequence(
    report: &WelderReport,
    workspace_root: &Path,
    cli: &Cli,
) -> Result<IgnitionWorkspaceHint> {
    if exe_looks_like_downloads_or_tmp() {
        let show = Confirm::new("Your eris binary looks like it lives under Downloads or /tmp. Show a short layout tip?")
            .with_default(true)
            .prompt()
            .map_err(inquire_to_fcp)?;
        if show {
            print_binary_layout_tip();
        }
    }

    let cwd_display = workspace_root.display().to_string();
    let cwd_prompt = format!(
        "Use this directory as your vault root?\n  {cwd_display}\n(Config and notes live here; you normally `cd` here before `eris chat`.)"
    );
    let use_cwd = Confirm::new(&cwd_prompt)
        .with_default(true)
        .prompt()
        .map_err(inquire_to_fcp)?;

    if !use_cwd {
        let home = std::env::var("HOME").unwrap_or_else(|_| "$HOME".to_string());
        eprintln!();
        eprintln!("Create a vault directory, enter it, then start again, for example:");
        eprintln!("  mkdir -p \"{home}/eris/vaults/MyVault\" && cd \"{home}/eris/vaults/MyVault\" && eris chat");
        eprintln!();
        return Err(FcpError::Config(
            "First-run vault: choose a directory with `cd`, then run `eris chat` again from there."
                .into(),
        ));
    }

    let default_ws = if cli.workspace != "default" {
        sanitize_workspace_id(&cli.workspace)
    } else {
        sanitize_workspace_id(&folder_hint(workspace_root))
    };

    let workspace = Text::new("Workspace id (Qdrant partition / ephemeral suffix):")
        .with_default(&default_ws)
        .prompt()
        .map_err(inquire_to_fcp)?;
    let workspace = sanitize_workspace_id(workspace.trim());

    if report.qdrant_blocked() {
        eprintln!();
        eprintln!("Qdrant is required (require_semantic_brain) but not reachable and neither");
        eprintln!("the `qdrant` binary nor `docker` was found on PATH. Eris can auto-start Qdrant");
        eprintln!("via Docker (`qdrant/qdrant:latest`) or a native `qdrant` install.");
        eprintln!("Install Docker or Qdrant, then run `eris chat` again.");
        eprintln!("  macOS Docker: https://docs.docker.com/desktop/setup/install/mac-install/");
        eprintln!("  Linux Docker: https://docs.docker.com/engine/install/");
        return Err(FcpError::NetworkFault(
            "First-run setup: cannot satisfy require_semantic_brain without Qdrant or Docker."
                .into(),
        ));
    }

    if !report.ollama_api_ok {
        eprintln!();
        eprintln!("Ollama HTTP API is not reachable at the configured host.");
        eprintln!("  Install: https://ollama.com  (macOS/Linux)");
        offer_install_help(report)?;
    } else {
        tracing::info!(target: "fcp.setup", "Welder: Ollama API reachable");
    }

    if !report.qdrant_tcp_ok && report.require_semantic_brain {
        tracing::info!(
            target: "fcp.setup",
            "Welder: Qdrant not up yet; peripherals will try native binary or Docker"
        );
    }

    Ok(IgnitionWorkspaceHint { workspace })
}

fn offer_install_help(report: &WelderReport) -> Result<()> {
    if report.ollama_cli {
        eprintln!("The `ollama` CLI is on PATH; Eris will try to start `ollama serve` if the API stays down.");
        return Ok(());
    }

    if cfg!(target_os = "macos") {
        eprintln!("On macOS you can install the CLI with Homebrew:  brew install ollama");
        let run = Confirm::new(
            "Try `brew install ollama` now? (uses the network; brew may ask for a password on some setups)",
        )
        .with_default(false)
        .prompt()
        .map_err(inquire_to_fcp)?;
        if run {
            let status = std::process::Command::new("brew")
                .args(["install", "ollama"])
                .status();
            match status {
                Ok(s) if s.success() => eprintln!("brew install ollama finished successfully."),
                Ok(s) => eprintln!("brew exited with status {s:?} — start Ollama manually if needed."),
                Err(e) => eprintln!("Could not run brew: {e}"),
            }
        }
    } else if cfg!(target_os = "linux") {
        eprintln!("On Linux follow upstream install steps: https://ollama.com/download/linux");
    }
    Ok(())
}

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize_workspace_id;

    #[test]
    fn sanitize_strips_and_folds() {
        assert_eq!(sanitize_workspace_id("My Vault!"), "My_Vault");
        assert_eq!(sanitize_workspace_id("___"), "default");
        assert_eq!(sanitize_workspace_id("ok-name"), "ok-name");
    }
}

fn inquire_to_fcp(e: inquire::InquireError) -> FcpError {
    match e {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            FcpError::Cancellation("First-run welder cancelled".into())
        }
        _ => FcpError::Config(format!("Welder prompt error: {e}")),
    }
}
