use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};
use tokio_util::sync::CancellationToken;

pub async fn execute_command(cmd: Commands, cancel_token: CancellationToken) -> Result<()> {
    // Acknowledge the parameter to prevent unused variable warnings while not doing anything yet
    let _ = cancel_token;
    
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
    use std::time::Duration;

    #[test]
    fn test_tool_non_existent_routing() {
        let cmd = Commands::Tool {
            name: "non_existent_tool".to_string(),
            args: "{}".to_string(),
        };
        // We can use a minimal block_on here since execute_command doesn't actually await yet
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(execute_command(cmd, CancellationToken::new()));
        
        assert!(result.is_err());
        match result.unwrap_err() {
            FcpError::Config(msg) => {
                assert!(msg.contains("non_existent_tool"));
            }
            _ => panic!("Expected Config error for non-existent tool"),
        }
    }

    #[tokio::test]
    async fn test_cancellation_token_yields() {
        // This test sets up the foundation for checking if background tasks properly honor the CancellationToken.
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        // Spawn a background task that cancels the token
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            token_clone.cancel();
        });

        // Normally `execute_command` would start a long-running process (like Chat) that we would select! over.
        // For now, we simulate waiting for it by asserting the token is cancelled.
        cancel_token.cancelled().await;
        assert!(cancel_token.is_cancelled());
        
        // Ensure execution handles the exit without panic
        let cmd = Commands::Chat;
        let result = execute_command(cmd, cancel_token).await;
        assert!(result.is_ok());
    }
}
