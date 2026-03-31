use crate::executive::cli::Commands;
use crate::executive::error::{FcpError, Result};
use tokio_util::sync::CancellationToken;

pub async fn execute_command(cmd: Commands, cancel_token: CancellationToken) -> Result<()> {
    // Acknowledge the parameter to prevent unused variable warnings while not doing anything yet
    let _ = cancel_token;
    
    match cmd {
                Commands::Chat => {
            use crate::ui::terminal::{setup_terminal, restore_terminal};
            use crate::ui::TuiApp;
            use tokio::sync::mpsc;
            use crate::orchestrator::core::Orchestrator;
            use crate::engine::ollama::OllamaClient;
            use crate::memory::ephemeral::EphemeralMemory;
            use crate::tools::Gatekeeper;
            use std::sync::Arc;
            use std::path::PathBuf;
            use ollama_rs::Ollama;

            let vault_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

            // 0. Ignition Sequence
            let seal_path = vault_root.join(".fcp_seal");
            if !seal_path.exists() {
                crate::executive::ignition::run_ignition_sequence(&vault_root).await?;
            }

            // 1. Setup channels
            let (tui_tx, tui_rx) = mpsc::channel(100);
            let (action_tx, mut action_rx) = mpsc::channel(100);

            // 2. Setup Terminal
            let terminal = setup_terminal()?;

            // 3. Spawn Orchestrator
            let config = Arc::new(crate::config::AppConfig::default());
            let client = Ollama::new("http://localhost".to_string(), 11434);
            let engine = OllamaClient::new(client, config.clone());
            let ephemeral = Arc::new(EphemeralMemory::new("default".to_string()));
            let mut gatekeeper = Gatekeeper::new();
            let vault_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            
            let read_limit = (config.llm_context_window as f32 * config.vault_read_ratio) as usize;
            gatekeeper.register(Arc::new(crate::tools::vault::VaultReadTool {
                workspace_root: vault_root.clone(),
                read_limit,
            }));
            
            let (interrupt_tx, interrupt_rx) = tokio::sync::watch::channel(());
            let _ = interrupt_tx; // Keep alive
            
            let mut orchestrator = Orchestrator::new(
                engine,
                gatekeeper,
                ephemeral,
                &vault_root,
                "default",
                3,
                5,
                0.8,
                4096,
                interrupt_rx,
                Some(tui_tx.clone()),
            );

            let tui_tx_err = tui_tx.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        Some(msg) = action_rx.recv() => {
                            orchestrator.chat_stack.push(crate::engine::Message {
                                role: "user".to_string(),
                                content: msg,
                            });
                            orchestrator.state = crate::orchestrator::state::AgentState::Chat;
                            if let Err(e) = orchestrator.step(None).await {
                                let err_msg = format!("[FATAL ERROR] Orchestrator halted: {}", e);
                                let _ = tui_tx_err.send(crate::ui::events::TuiEvent::IncomingMessage(err_msg)).await;
                                break;
                            }
                        }
                    }
                }
            });

            // 4. Run TUI App
            let mut app = TuiApp::new(tui_rx, action_tx);
            let result = app.run(terminal).await;

            // 5. Teardown
            restore_terminal()?;
            result
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
        let cmd = Commands::Run { prompt: "test".to_string() };
        let result = execute_command(cmd, cancel_token).await;
        assert!(result.is_ok());
    }
}
