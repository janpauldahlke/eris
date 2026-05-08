#![deny(clippy::unwrap_used)]
// `unwrap` in `#[test]` is allowed by project policy; clippy still checks non-test code.
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::executive::cli::Cli;
use crate::executive::router::execute_command;

pub mod config;
pub mod engine;
pub mod executive;
pub mod generated;
pub mod ingest;
pub mod memory;
pub mod orchestrator;
pub mod presentation;
pub mod skills;
pub mod telemetry;
pub mod tools;
pub mod ui;
pub mod util;
pub mod vault_layout;
pub mod workspace;

#[tokio::main]
async fn main() -> ExitCode {
    // 1. Parse CLI arguments
    // Uses clap's native parse() to properly handle --help and exit correctly
    let cli = Cli::parse();

    // 2. Load configuration first (`.fcp/config.toml` relative to cwd).
    let config = match AppConfig::load(cli.clone()) {
        Ok(c) => std::sync::Arc::new(c),
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // 3. Telemetry under the same directory as chat (launch cwd).
    let log_vault_root = config.active_vault();
    let _guard = match crate::telemetry::logger::init_tracing(&log_vault_root) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Failed to initialize telemetry: {}", e);
            return ExitCode::FAILURE;
        }
    };

    tracing::info!("Starting FCP Subconscious Orchestrator...");
    tracing::info!(
        model = %config.model_name,
        workspace = %config.workspace,
        vault_dir = %log_vault_root.display(),
        "Configuration loaded"
    );

    // 5. Pre-Flight Checks
    if let Err(e) = crate::telemetry::preflight::run_preflight_checks(&cli.command, &config).await {
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    // 6. Global Kill Switch (CancellationToken) setup
    let cancel_token = CancellationToken::new();
    let token_clone = cancel_token.clone();

    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            tracing::warn!("SIGINT received, triggering global shutdown...");
            token_clone.cancel();
        }
    });

    // 7. Route Execution
    if let Err(e) = execute_command(cli, config, cancel_token).await {
        tracing::error!("{}", e);
        // It's also printed to stderr as a structured error trace
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
