#![deny(clippy::unwrap_used)]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use tokio_util::sync::CancellationToken;

use crate::executive::cli::Cli;
use crate::executive::router::execute_command;

pub mod executive;
pub mod telemetry;
pub mod config;
pub mod workspace;
pub mod engine;
pub mod memory;
pub mod tools;
pub mod orchestrator;
pub mod ui;

#[tokio::main]
async fn main() -> ExitCode {
    // 1. Parse CLI arguments
    // Uses clap's native parse() to properly handle --help and exit correctly
    let cli = Cli::parse();

    // 2. Resolve Workspace Root
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    // 3. Initialize Telemetry (Black Box)
    let _guard = match crate::telemetry::logger::init_tracing(&workspace_root) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("Failed to initialize telemetry: {}", e);
            return ExitCode::FAILURE;
        }
    };

    tracing::info!("Starting FCP Subconscious Orchestrator...");

    // 4. Pre-Flight Checks
    if let Err(e) = crate::telemetry::preflight::run_preflight_checks().await {
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    // 5. Global Kill Switch (CancellationToken) setup
    let cancel_token = CancellationToken::new();
    let token_clone = cancel_token.clone();

    // Set up the ctrl_c signal trap
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            tracing::warn!("SIGINT received, triggering global shutdown...");
            token_clone.cancel();
        }
    });

    // 4. Route Execution
    // Pass the cancel_token down to the router so long-running processes can yield when it is cancelled.
    if let Err(e) = execute_command(cli.command, cancel_token).await {
        tracing::error!("{}", e);
        // It's also printed to stderr as a structured error trace
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
