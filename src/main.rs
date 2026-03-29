#![deny(clippy::unwrap_used)]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use tokio_util::sync::CancellationToken;

use crate::executive::cli::Cli;
use crate::executive::router::execute_command;
use crate::telemetry::logger::{init_tracing, LogTarget};

pub mod executive;
pub mod telemetry;
pub mod config;

#[tokio::main]
async fn main() -> ExitCode {
    // 1. Parse CLI arguments
    // Uses clap's native parse() to properly handle --help and exit correctly
    let cli = Cli::parse();

    // 2. Initialize Telemetry (Black Box)
    // For now, always to Stderr, later we can add file appender logic
    if let Err(e) = init_tracing(cli.verbose, LogTarget::Stderr) {
        eprintln!("Failed to initialize telemetry: {}", e);
        return ExitCode::FAILURE;
    }

    tracing::info!("Starting FCP Subconscious Orchestrator...");

    // 3. Global Kill Switch (CancellationToken) setup
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
