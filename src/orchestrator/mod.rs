pub mod state;
pub mod context;

use crate::engine::LlmEngine;
use crate::tools::Gatekeeper;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::{AgentState, LoopDirective, LlmResponse, LoopAction};
use crate::orchestrator::context::ContextAssembler;
use std::sync::Arc;
use std::path::Path;

pub struct Orchestrator<E: LlmEngine> {
    pub state: AgentState,
    pub engine: E,
    pub gatekeeper: Gatekeeper,
    pub ephemeral: Arc<EphemeralMemory>,
    pub context_assembler: ContextAssembler,

    // Bounds
    pub max_recovery_attempts: u8,
    pub max_tool_rounds: u8,
    pub condensation_threshold: f32,
    pub num_ctx: usize,

    // Live Loop State
    pub recovery_count: u8,
    pub tool_rounds: u8,
}

impl<E: LlmEngine> Orchestrator<E> {
    pub fn new(
        engine: E,
        gatekeeper: Gatekeeper,
        ephemeral: Arc<EphemeralMemory>,
        vault_root: &Path,
        workspace: &str,
        max_recovery_attempts: u8,
        max_tool_rounds: u8,
        condensation_threshold: f32,
        num_ctx: usize,
    ) -> Self {
        Self {
            state: AgentState::Idle,
            engine,
            gatekeeper,
            ephemeral,
            context_assembler: ContextAssembler::new(vault_root, workspace),
            max_recovery_attempts,
            max_tool_rounds,
            condensation_threshold,
            num_ctx,
            recovery_count: 0,
            tool_rounds: 0,
        }
    }

    pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
        let response: LlmResponse = match serde_json::from_str(response_json) {
            Ok(res) => res,
            Err(e) => return LoopDirective::RecoverFromFuckup(e.to_string()),
        };

        match response.status {
            LoopAction::ContinueTask => {
                if response.tool_calls.is_empty() {
                    LoopDirective::RecoverFromFuckup("CONTINUE_TASK requires tool_calls".to_string())
                } else {
                    LoopDirective::ExecuteTools(response.tool_calls)
                }
            }
            LoopAction::WaitForUser => {
                LoopDirective::HaltAndAwaitInput(response.message_to_user)
            }
            LoopAction::InitiateReflection => {
                self.state = AgentState::Reflect;
                LoopDirective::ShiftToReflection
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Message, EngineResponse};
    use async_trait::async_trait;
    use tokio::sync::mpsc;
    use crate::executive::error::Result;

    #[derive(Clone)]
    struct MockEngine;

    #[async_trait]
    impl LlmEngine for MockEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>
        ) -> Result<EngineResponse> {
            Ok(EngineResponse {
                content: "mock".to_string(),
                prompt_tokens: 0,
                generated_tokens: 0,
            })
        }
    }

    #[test]
    fn test_orchestrator_initialization() {
        let engine = MockEngine;
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");

        let orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            "test_ws",
            3,
            5,
            0.8,
            4096,
        );

        assert_eq!(orchestrator.state, AgentState::Idle);
        assert_eq!(orchestrator.recovery_count, 0);
        assert_eq!(orchestrator.tool_rounds, 0);
        assert_eq!(orchestrator.max_recovery_attempts, 3);
        assert_eq!(orchestrator.max_tool_rounds, 5);
        assert_eq!(orchestrator.condensation_threshold, 0.8);
        assert_eq!(orchestrator.num_ctx, 4096);
    }

    fn setup_orchestrator() -> Orchestrator<MockEngine> {
        let engine = MockEngine;
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        Orchestrator::new(engine, gatekeeper, ephemeral, vault_root, "test_ws", 3, 5, 0.8, 4096)
    }

    #[test]
    fn test_router_valid_tool_call() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "status": "CONTINUE_TASK",
            "tool_calls": [{ "name": "foo", "args": {} }]
        }"#;
        
        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::ExecuteTools(tools) => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "foo");
            }
            _ => panic!("Expected ExecuteTools"),
        }
    }

    #[test]
    fn test_router_missing_tools_yields_fuckup() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "status": "CONTINUE_TASK",
            "tool_calls": []
        }"#;

        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::RecoverFromFuckup(_) => {}
            _ => panic!("Expected RecoverFromFuckup"),
        }
    }

    #[test]
    fn test_router_invalid_json_yields_fuckup() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{"status": "BAD_JSON"#;
        
        let directive = orchestrator.process_llm_response(json);
        match directive {
            LoopDirective::RecoverFromFuckup(msg) => {
                assert!(!msg.is_empty());
            }
            _ => panic!("Expected RecoverFromFuckup"),
        }
    }

    #[test]
    fn test_router_initiate_reflection_mutates_state() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "status": "INITIATE_REFLECTION"
        }"#;
        
        let directive = orchestrator.process_llm_response(json);
        assert_eq!(directive, LoopDirective::ShiftToReflection);
        assert_eq!(orchestrator.state, AgentState::Reflect);
    }
}
