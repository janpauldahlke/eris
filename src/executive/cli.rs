use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

use super::error::{FcpError, Result};

#[derive(Parser, Debug, Clone)]
#[command(name = "fcp", version, about = "The Unified Dreadnought: Local SLM Orchestrator", disable_version_flag = true)]
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
    /// Boot the Layer 2 Subconscious and enter the interactive loop
    Chat,

    /// Execute a single-shot prompt and exit (useful for bash piping)
    Run { prompt: String },

    /// Bypass Layer 1 entirely and manually invoke a Layer 2 tool
    Tool {
        name: String,
        args: String,
    },
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
        let args = vec!["fcp", "-w", "default", "chat"];
        let cli = parse_from(args).unwrap();
        assert_eq!(cli.vault, None);
        assert_eq!(cli.workspace, "default");
        assert_eq!(cli.command, Commands::Chat);
    }

    #[test]
    fn test_cli_verbose_stacking() {
        let args1 = vec!["fcp", "-V", "chat"];
        let cli1 = parse_from(args1).unwrap();
        assert_eq!(cli1.verbose, 1);

        let args2 = vec!["fcp", "-VV", "chat"];
        let cli2 = parse_from(args2).unwrap();
        assert_eq!(cli2.verbose, 2);
    }

    #[test]
    fn test_cli_tool_subcommand() {
        let args = vec!["fcp", "tool", "memory:query", r#"{"query": "test"}"#];
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
        let args = vec!["fcp", "--workspace", "isolated_env", "chat"];
        let cli = parse_from(args).unwrap();
        assert_eq!(cli.workspace, "isolated_env");
    }

    #[test]
    fn test_cli_tool_args_as_single_string() {
        let args = vec!["fcp", "tool", "memory:query", r#"{"q": "test"}"#];
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
        let args = vec!["fcp", "non-existent-command"];
        let result = parse_from(args);
        assert!(result.is_err());
        match result.unwrap_err() {
            FcpError::Config(_) => (),
            _ => panic!("Expected Config error on invalid CLI arguments"),
        }
    }
}
