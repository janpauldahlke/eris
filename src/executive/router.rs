use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};

pub fn execute_command(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Chat => {
            // Placeholder for chat loop
            Ok(())
        }
        Commands::Run { prompt } => {
            // Placeholder for single-shot execution
            let _ = prompt;
            Ok(())
        }
        Commands::Tool { name, args } => {
            // Route to specific tools
            let _ = args;
            match name.as_str() {
                "memory:query" => Ok(()),
                _ => Err(FcpError::Config(format!("Tool not found: {}", name))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_non_existent_routing() {
        let cmd = Commands::Tool {
            name: "non_existent_tool".to_string(),
            args: vec![],
        };
        let result = execute_command(cmd);
        
        assert!(result.is_err());
        match result.unwrap_err() {
            FcpError::Config(msg) => {
                assert!(msg.contains("non_existent_tool"));
            }
            _ => panic!("Expected Config error for non-existent tool"),
        }
    }
}
