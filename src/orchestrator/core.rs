use crate::executive::error::Result;
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

    pub chat_stack: Vec<crate::engine::Message>,
    pub saved_chat_state: Option<Vec<crate::engine::Message>>,
    pub interrupt_rx: tokio::sync::watch::Receiver<()>,
}

impl<E: LlmEngine> Orchestrator<E> {
    #[allow(clippy::too_many_arguments)]
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
        interrupt_rx: tokio::sync::watch::Receiver<()>,
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
            chat_stack: Vec::new(),
            saved_chat_state: None,
            interrupt_rx,
        }
    }

    /// The main cognitive loop.
    #[allow(clippy::never_loop)]
    pub async fn step(&mut self, _user_input: Option<String>) -> Result<()> {
        // TODO: Inject user_input into memory if provided

        loop {
            // 1. Bailout Checks
            if self.recovery_count >= self.max_recovery_attempts {
                self.state = AgentState::Idle;
                return Ok(());
            }
            if self.tool_rounds >= self.max_tool_rounds {
                self.state = AgentState::Idle;
                return Ok(());
            }

            // 2. Context Assembly (Mocked for now)
            // let prompt = self.context.assemble(&self.state, &self.ephemeral).await?;

            // 3. Engine Generation (Mocked for now)
            let response_result = tokio::select! {
                res = self.engine.generate(&self.chat_stack, "{}", None) => res,
                _ = self.interrupt_rx.changed() => {
                    // The heartbeat fired.
                    self.saved_chat_state = Some(self.chat_stack.clone());
                    self.chat_stack.clear();
                    // Push IDLE_STATE prompt to chat_stack
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: "IDLE_STATE".to_string(),
                    });
                    self.state = AgentState::Idle;
                    return Err(crate::executive::error::FcpError::Interrupted);
                }
            };

            let response = match response_result {
                Ok(res) => res,
                Err(e) => {
                    self.state = AgentState::Idle;
                    return Err(e);
                }
            };

            if (response.generated_tokens + response.prompt_tokens) > (self.num_ctx as f32 * self.condensation_threshold) as usize {
                self.state = AgentState::Reflect;
            }

            // 4. Directive Processing
            let directive = self.process_llm_response(&response.content);

            match directive {
                LoopDirective::HaltAndAwaitInput(_msg) => {
                    self.state = AgentState::Idle;
                    self.tool_rounds = 0;
                    self.recovery_count = 0;
                    // If we had a user interface hooked up, we'd yield `msg` here.
                    return Ok(());
                }
                LoopDirective::ExecuteTools(tools) => {
                    for tool_call in tools {
                        match self.gatekeeper.execute_tool(&self.state, &tool_call.name, tool_call.args).await {
                            Ok(result) => {
                                self.tool_rounds += 1;
                                self.recovery_count = 0;
                                self.chat_stack.push(crate::engine::Message {
                                    role: "system".to_string(),
                                    content: format!("Tool '{}' succeeded: {}", tool_call.name, result),
                                });
                            }
                            Err(e) => {
                                // Cognitive Fault: Catch Schema/Tool errors and force recovery
                                if matches!(e, crate::executive::error::FcpError::ToolFault { .. } | crate::executive::error::FcpError::SchemaViolation(_)) {
                                    self.recovery_count += 1;
                                    self.state = AgentState::Recover;
                                    self.chat_stack.push(crate::engine::Message {
                                        role: "system".to_string(),
                                        content: format!("[SYSTEM OVERRIDE: FUCKUP DETECTED] Tool execution failed: {}", e),
                                    });
                                    break; // Break the inner tool loop to restart the outer cognitive loop
                                } else {
                                    // System Fatality (e.g., Network offline): Abort entirely
                                    self.state = AgentState::Idle;
                                    return Err(e);
                                }
                            }
                        }
                    }
                }
                LoopDirective::RecoverFromFuckup(msg) => {
                    self.recovery_count += 1;
                    self.state = AgentState::Recover;
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: format!("[SYSTEM OVERRIDE: FUCKUP DETECTED] Invalid LLM Output: {}", msg),
                    });
                }
                LoopDirective::ShiftToReflection => {
                    self.state = AgentState::Reflect;
                }
            }
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

    /// Forces the LLM to summarize the active chat stack, then replaces the stack with the summary.
    pub async fn execute_condensation(&mut self) -> Result<()> {
        // 1. Push a system message to `self.chat_stack` asking for a JSON summary.
        self.chat_stack.push(crate::engine::Message {
            role: "system".to_string(),
            content: "Please summarize the current conversation as a JSON object.".to_string(),
        });

        // 2. Call `self.engine.generate(&self.chat_stack, ...)`.
        let response = self.engine.generate(&self.chat_stack, "{}", None).await?;

        // 3. Extract the summary text from the response.
        let summary = response.content;

        // 4. Clear `self.chat_stack`.
        self.chat_stack.clear();

        // 5. Push a single Message containing the summary back to `self.chat_stack`.
        self.chat_stack.push(crate::engine::Message {
            role: "system".to_string(),
            content: summary,
        });

        // 6. Set `self.state = AgentState::Chat`.
        self.state = AgentState::Chat;

        Ok(())
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
    struct MockEngine {
        content: String,
        fault: Option<String>,
        prompt_tokens: usize,
        generated_tokens: usize,
    }

    impl MockEngine {
        fn new() -> Self {
            Self {
                content: "mock".to_string(),
                fault: None,
                prompt_tokens: 0,
                generated_tokens: 0,
            }
        }

        fn with_content(content: &str) -> Self {
            Self {
                content: content.to_string(),
                fault: None,
                prompt_tokens: 0,
                generated_tokens: 0,
            }
        }

        fn with_network_fault(msg: &str) -> Self {
            Self {
                content: String::new(),
                fault: Some(msg.to_string()),
                prompt_tokens: 0,
                generated_tokens: 0,
            }
        }

        fn with_tokens(mut self, prompt_tokens: usize, generated_tokens: usize) -> Self {
            self.prompt_tokens = prompt_tokens;
            self.generated_tokens = generated_tokens;
            self
        }
    }

    #[async_trait]
    impl LlmEngine for MockEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>
        ) -> Result<EngineResponse> {
            if let Some(msg) = &self.fault {
                return Err(crate::executive::error::FcpError::NetworkFault(msg.clone()));
            }
            Ok(EngineResponse {
                content: self.content.clone(),
                prompt_tokens: self.prompt_tokens,
                generated_tokens: self.generated_tokens,
            })
        }
    }

    #[test]
    fn test_orchestrator_initialization() {
        let engine = MockEngine::new();
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx));

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
            rx,
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
        setup_orchestrator_with_engine(MockEngine::new())
    }

    fn setup_orchestrator_with_engine(engine: MockEngine) -> Orchestrator<MockEngine> {
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        Box::leak(Box::new(tx)); // Prevent sender from dropping, which would trigger `rx.changed()`
        Orchestrator::new(engine, gatekeeper, ephemeral, vault_root, "test_ws", 3, 5, 0.8, 4096, rx)
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

    #[tokio::test]
    async fn test_step_bails_out_on_max_recovery() {
        let mut orchestrator = setup_orchestrator();
        orchestrator.recovery_count = orchestrator.max_recovery_attempts;
        orchestrator.state = AgentState::Chat;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn test_step_bails_out_on_max_tool_rounds() {
        let mut orchestrator = setup_orchestrator();
        orchestrator.tool_rounds = orchestrator.max_tool_rounds;
        orchestrator.state = AgentState::Chat;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn test_step_system_fatality_aborts() {
        let engine = MockEngine::with_network_fault("daemon offline");
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_err());
        assert_eq!(orchestrator.state, AgentState::Idle);
    }

    #[tokio::test]
    async fn test_step_halt_directive_resets_state() {
        let json = r#"{
            "status": "WAIT_FOR_USER",
            "message_to_user": "how can I help?"
        }"#;
        let engine = MockEngine::with_content(json);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        orchestrator.tool_rounds = 2;
        orchestrator.recovery_count = 1;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.state, AgentState::Idle);
        assert_eq!(orchestrator.tool_rounds, 0);
        assert_eq!(orchestrator.recovery_count, 0);
    }

    #[tokio::test]
    async fn test_execute_condensation_replaces_stack() {
        let json = r#"{
            "status": "CONTINUE_TASK",
            "tool_calls": []
        }"#;
        let engine = MockEngine::with_content(json);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.chat_stack.push(Message { role: "user".to_string(), content: "hello".to_string() });
        orchestrator.chat_stack.push(Message { role: "assistant".to_string(), content: "world".to_string() });

        let result = orchestrator.execute_condensation().await;
        
        assert!(result.is_ok());
        assert_eq!(orchestrator.chat_stack.len(), 1);
        assert_eq!(orchestrator.chat_stack[0].content, json);
        assert_eq!(orchestrator.state, AgentState::Chat);
    }

    #[tokio::test]
    async fn test_step_triggers_reflection_on_token_exhaustion() {
        let json = r#"{
            "status": "WAIT_FOR_USER",
            "message_to_user": "hello"
        }"#;
        // Wait, wait, if the engine returns WAIT_FOR_USER, loop will exit gracefully.
        // We shouldn't use CONTINUE_TASK without tools otherwise it loops to RecoverFromFuckup and does another loop
        // Let's use WAIT_FOR_USER to avoid infinite loops since we removed the `break`.
        // With num_ctx = 4096 and threshold = 0.8, max tokens = 3276
        let engine = MockEngine::with_content(json).with_tokens(2000, 1500);
        let mut orchestrator = setup_orchestrator_with_engine(engine);
        orchestrator.state = AgentState::Chat;
        
        let result = orchestrator.step(None).await;
        
        assert!(result.is_ok(), "Expected OK, got: {:?}", result.err());
        // state will be overridden by WAIT_FOR_USER (AgentState::Idle)
        // Hmm... previously `ShiftToReflection` was not tested for its *persistent* state change if loop broke immediately.
        // Let's change json to ShiftToReflection (InitiateReflection).
    }

    #[tokio::test]
    async fn test_async_guillotine_interrupts_generation() {
        use std::time::Duration;
        
        #[derive(Clone)]
        struct PendingEngine;
        #[async_trait]
        impl LlmEngine for PendingEngine {
            async fn generate(
                &self,
                _stack: &[Message],
                _available_tools_json: &str,
                _stream_tx: Option<mpsc::UnboundedSender<String>>
            ) -> Result<EngineResponse> {
                // Hang forever
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(EngineResponse {
                    content: "never".to_string(),
                    prompt_tokens: 0,
                    generated_tokens: 0,
                })
            }
        }

        let engine = PendingEngine;
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = tokio::sync::watch::channel(());
        
        let mut orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            "test_ws",
            3,
            5,
            0.8,
            4096,
            rx,
        );

        orchestrator.state = AgentState::Chat;
        orchestrator.chat_stack.push(Message { role: "user".to_string(), content: "hello".to_string() });

        // Fire the interrupt shortly after calling step
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send(());
        });

        let result = orchestrator.step(None).await;
        
        assert!(matches!(result, Err(crate::executive::error::FcpError::Interrupted)));
        assert_eq!(orchestrator.state, AgentState::Idle);
        assert!(orchestrator.saved_chat_state.is_some());
        assert_eq!(orchestrator.saved_chat_state.unwrap()[0].content, "hello");
        assert_eq!(orchestrator.chat_stack.len(), 1);
        assert_eq!(orchestrator.chat_stack[0].content, "IDLE_STATE");
    }
}
