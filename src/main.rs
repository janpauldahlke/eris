#![deny(clippy::unwrap_used)]
#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use crate::executive::cli::Cli;
use crate::executive::router::execute_command;
use crate::telemetry::logger::{init_tracing, LogTarget};

pub mod executive;
pub mod telemetry;

fn main() -> ExitCode {
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

    // 3. Route Execution
    if let Err(e) = execute_command(cli.command) {
        tracing::error!("{}", e);
        // It's also printed to stderr as a structured error trace
        eprintln!("{}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
