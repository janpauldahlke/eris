pub mod state;

use crate::engine::LlmEngine;
use crate::tools::Gatekeeper;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::AgentState;
use std::sync::Arc;

pub struct Orchestrator<E: LlmEngine> {
    pub state: AgentState,
    pub engine: E,
    pub gatekeeper: Gatekeeper,
    pub ephemeral: Arc<EphemeralMemory>,

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
            max_recovery_attempts,
            max_tool_rounds,
            condensation_threshold,
            num_ctx,
            recovery_count: 0,
            tool_rounds: 0,
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

        let orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
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
}
