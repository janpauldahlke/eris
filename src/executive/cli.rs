use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

use super::error::{FcpError, Result};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "eris",
    version,
    about = "The Unified Dreadnought: Local SLM Orchestrator",
    disable_version_flag = true
)]
pub struct Cli {
    /// Defines the active memory partition (isolates vector spaces)
    #[arg(short = 'w', long, env = "FCP_WORKSPACE", default_value = "default")]
    pub workspace: String,

    /// Overrides the AppConfig vault path
    #[arg(short = 'v', long, env = "FCP_VAULT")]
    pub vault: Option<PathBuf>,

    /// Increases telemetry verbosity (e.g., -V, -VV)
    #[arg(short = 'V', long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, PartialEq, Clone)]
pub enum Commands {
    /// Run model capability benchmarks (quality, reliability, speed)
    Benchmark {
        /// Benchmark suite to run (quick, standard, comprehensive)
        #[arg(short, long, default_value = "standard")]
        suite: String,

        /// Output format (table, json, markdown)
        #[arg(short, long, default_value = "table")]
        format: String,

        /// Compare with previous run or another model
        #[arg(long)]
        compare: bool,

        /// Compare two runs **in this vault only** by run ID (format: run-id-1..run-id-2).
        /// For reports under other vaults, use `--diff-files` instead.
        #[arg(long)]
        diff: Option<String>,

        /// Compare two saved report JSON files (any path). Baseline first, current second.
        /// Example: `--diff-files ../vaults/nemo/.fcp/benchmarks/a.json ../vaults/gemma/.fcp/benchmarks/b.json`
        #[arg(long = "diff-files", num_args = 2, value_names = ["BASELINE_JSON", "CURRENT_JSON"])]
        diff_files: Option<Vec<PathBuf>>,

        /// Compare **latest** saved reports from two vault directories (folder names relative to cwd).
        /// Example (from parent of `gemma/` and `nemo/`): `--diff-vaults gemma nemo`
        #[arg(
            long = "diff-vaults",
            alias = "diff-siblings",
            num_args = 2,
            value_names = ["BASELINE_VAULT_DIR", "CURRENT_VAULT_DIR"],
            conflicts_with_all = ["diff", "diff_files"],
            group = "action"
        )]
        diff_vaults: Option<Vec<String>>,

        /// List all benchmark runs for this vault
        #[arg(long, group = "action")]
        list: bool,

        /// Generate trend report from last N runs
        #[arg(long, group = "action")]
        trend: Option<usize>,

        /// Isolation mode (strict, relaxed, unsafe)
        #[arg(long, default_value = "strict")]
        isolation: String,

        /// Output file path (optional)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Disable dry-run mode (requires --i-understand-risks)
        #[arg(long)]
        no_dry_run: bool,

        /// Acknowledge risks of disabling dry-run
        #[arg(long)]
        i_understand_risks: bool,

        /// Keep benchmark artifacts for debugging
        #[arg(long)]
        no_cleanup: bool,
    },

    /// Boot the Layer 2 Subconscious and enter the interactive loop
    Chat {
        /// Browser UI: localhost HTTP server with SSE (see `web_bind_addr` / `web_port` in `.fcp/config.toml`).
        #[arg(long)]
        web: bool,
    },

    /// Execute a single-shot prompt and exit (useful for bash piping)
    Run { prompt: String },

    /// Bypass Layer 1 entirely and manually invoke a Layer 2 tool
    Tool { name: String, args: String },
}

pub fn parse_from<I, T>(itr: I) -> Result<Cli>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(itr).map_err(|e| FcpError::Config(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_default_vault() {
        // Pass `-w default` so this test does not depend on `FCP_WORKSPACE` in the environment.
        let args = vec!["eris", "-w", "default", "chat"];
        let cli = parse_from(args).unwrap();
        assert_eq!(cli.vault, None);
        assert_eq!(cli.workspace, "default");
        assert_eq!(cli.command, Commands::Chat { web: false });
    }

    #[test]
    fn test_cli_chat_web_flag() {
        let args = vec!["eris", "-w", "default", "chat", "--web"];
        let cli = parse_from(args).unwrap();
        assert_eq!(cli.command, Commands::Chat { web: true });
    }

    #[test]
    fn test_cli_verbose_stacking() {
        let args1 = vec!["eris", "-V", "chat"];
        let cli1 = parse_from(args1).unwrap();
        assert_eq!(cli1.verbose, 1);

        let args2 = vec!["eris", "-VV", "chat"];
        let cli2 = parse_from(args2).unwrap();
        assert_eq!(cli2.verbose, 2);
    }

    #[test]
    fn test_cli_tool_subcommand() {
        let args = vec!["eris", "tool", "memory:query", r#"{"query": "test"}"#];
        let cli = parse_from(args).unwrap();
        if let Commands::Tool { name, args } = cli.command {
            assert_eq!(name, "memory:query");
            assert_eq!(args, r#"{"query": "test"}"#);
        } else {
            panic!("Expected Tool subcommand");
        }
    }

    #[test]
    fn test_cli_workspace_propagation() {
        let args = vec!["eris", "--workspace", "isolated_env", "chat"];
        let cli = parse_from(args).unwrap();
        assert_eq!(cli.workspace, "isolated_env");
    }

    #[test]
    fn test_cli_tool_args_as_single_string() {
        let args = vec!["eris", "tool", "memory:query", r#"{"q": "test"}"#];
        let cli = parse_from(args).unwrap();
        if let Commands::Tool { name, args } = cli.command {
            assert_eq!(name, "memory:query");
            assert_eq!(args, r#"{"q": "test"}"#);
        } else {
            panic!("Expected Tool subcommand");
        }
    }

    #[test]
    fn test_cli_parse_error_returns_config_fault() {
        let args = vec!["eris", "non-existent-command"];
        let result = parse_from(args);
        assert!(result.is_err());
        match result.unwrap_err() {
            FcpError::Config(_) => (),
            _ => panic!("Expected Config error on invalid CLI arguments"),
        }
    }

    // Benchmark command tests
    #[test]
    fn test_cli_benchmark_default() {
        let args = vec!["eris", "benchmark"];
        let cli = parse_from(args).unwrap();
        
        if let Commands::Benchmark { suite, format, isolation, no_dry_run, .. } = cli.command {
            assert_eq!(suite, "standard");
            assert_eq!(format, "table");
            assert_eq!(isolation, "strict");
            assert!(!no_dry_run); // Default is dry-run
        } else {
            panic!("Expected Benchmark subcommand");
        }
    }

    #[test]
    fn test_cli_benchmark_suite_selection() {
        let args = vec!["eris", "benchmark", "--suite", "quick"];
        let cli = parse_from(args).unwrap();
        
        if let Commands::Benchmark { suite, .. } = cli.command {
            assert_eq!(suite, "quick");
        } else {
            panic!("Expected Benchmark subcommand");
        }
    }

    #[test]
    fn test_cli_benchmark_output_format() {
        let args = vec!["eris", "benchmark", "--format", "json"];
        let cli = parse_from(args).unwrap();
        
        if let Commands::Benchmark { format, .. } = cli.command {
            assert_eq!(format, "json");
        } else {
            panic!("Expected Benchmark subcommand");
        }
    }

    #[test]
    fn test_cli_benchmark_list_mode() {
        let args = vec!["eris", "benchmark", "--list"];
        let cli = parse_from(args).unwrap();
        
        if let Commands::Benchmark { list, .. } = cli.command {
            assert!(list);
        } else {
            panic!("Expected Benchmark subcommand");
        }
    }

    #[test]
    fn test_cli_benchmark_isolation_mode() {
        let args = vec!["eris", "benchmark", "--isolation", "relaxed"];
        let cli = parse_from(args).unwrap();
        
        if let Commands::Benchmark { isolation, .. } = cli.command {
            assert_eq!(isolation, "relaxed");
        } else {
            panic!("Expected Benchmark subcommand");
        }
    }

    #[test]
    fn test_cli_benchmark_diff_vaults() {
        let args = vec![
            "eris",
            "benchmark",
            "--diff-vaults",
            "gemma",
            "nemo",
        ];
        let cli = parse_from(args).unwrap();

        if let Commands::Benchmark {
            diff_vaults,
            suite,
            ..
        } = cli.command
        {
            assert_eq!(
                diff_vaults,
                Some(vec!["gemma".to_string(), "nemo".to_string()])
            );
            assert_eq!(suite, "standard");
        } else {
            panic!("Expected Benchmark subcommand");
        }
    }
}
