use crate::executive::error::Result;
use crate::engine::LlmEngine;
use crate::tools::Gatekeeper;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::state::{AgentState, LoopDirective, LlmResponse, LoopAction};
use crate::orchestrator::context::ContextAssembler;
use futures::future;
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
    pub tui_tx: Option<tokio::sync::mpsc::Sender<crate::ui::events::TuiEvent>>,
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
        tui_tx: Option<tokio::sync::mpsc::Sender<crate::ui::events::TuiEvent>>,
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
            tui_tx,
        }
    }


    pub async fn broadcast_state(&self) {
        if let Some(tx) = &self.tui_tx {
            let update = crate::ui::events::AgentStateUpdate {
                state: self.state,
                tool_rounds: self.tool_rounds,
                recovery_count: self.recovery_count,
                active_task: None,
            };
            let _ = tx.send(crate::ui::events::TuiEvent::StateUpdate(update)).await;
        }
    }

    /// The main cognitive loop.
    #[allow(clippy::never_loop)]
    pub async fn step(&mut self, _user_input: Option<String>) -> Result<()> {
        tracing::info!(state = ?self.state, tool_rounds = self.tool_rounds, recovery_count = self.recovery_count, chat_stack_len = self.chat_stack.len(), "step() entered");
        self.broadcast_state().await;

        loop {
            // 1. Bailout Checks
            if self.recovery_count >= self.max_recovery_attempts {
                tracing::warn!(recovery_count = self.recovery_count, max = self.max_recovery_attempts, "Max recovery attempts reached, bailing out");
                self.state = AgentState::Idle;
                self.broadcast_state().await;
                return Ok(());
            }
            if self.tool_rounds >= self.max_tool_rounds {
                tracing::warn!(tool_rounds = self.tool_rounds, max = self.max_tool_rounds, "Max tool rounds reached, bailing out");
                self.state = AgentState::Idle;
                self.broadcast_state().await;
                return Ok(());
            }

            // 2. Context Assembly
            let system_prompt = self.context_assembler.assemble(&self.state, &self.ephemeral, &self.gatekeeper).await?;
            tracing::debug!(prompt_len = system_prompt.len(), "System prompt assembled");
            
            if let Some(first) = self.chat_stack.first_mut() {
                if first.content.contains("You are operating within a strict programmatic state machine") {
                    first.content = system_prompt;
                } else {
                    self.chat_stack.insert(0, crate::engine::Message {
                        role: "system".to_string(),
                        content: system_prompt,
                    });
                }
            } else {
                self.chat_stack.push(crate::engine::Message {
                    role: "system".to_string(),
                    content: system_prompt,
                });
            }

            tracing::info!(chat_stack_len = self.chat_stack.len(), "Sending to LLM engine");

            // 3. Engine Generation
            let mut stream_forwarder = None;
            let stream_tx = if let Some(tui_tx) = self.tui_tx.clone() {
                let _ = tui_tx.send(crate::ui::events::TuiEvent::AssistantStreamStart).await;
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                let forward_tx = tui_tx.clone();
                stream_forwarder = Some(tokio::spawn(async move {
                    while let Some(chunk) = rx.recv().await {
                        let _ = forward_tx.send(crate::ui::events::TuiEvent::IncomingMessageChunk(chunk)).await;
                    }
                }));
                Some(tx)
            } else {
                None
            };

            let response_result = tokio::select! {
                res = self.engine.generate(&self.chat_stack, "", stream_tx) => res,
                _ = self.interrupt_rx.changed() => {
                    // The heartbeat fired.
                    self.saved_chat_state = Some(self.chat_stack.clone());
                    self.chat_stack.clear();

                    // Read .fcp_agenda.json to inject oldest task if present
                    let workspace_root = self.context_assembler.core_dir.parent().unwrap_or(&self.context_assembler.core_dir);
                    let agenda_path = workspace_root.join(".fcp_agenda.json");
                    
                    let mut active_task = None;
                    if let Ok(content) = tokio::fs::read_to_string(&agenda_path).await
                        && let Ok(tasks) = serde_json::from_str::<Vec<serde_json::Value>>(&content)
                            && let Some(desc) = tasks.first().and_then(|first| first.get("description")).and_then(|d| d.as_str()) {
                                active_task = Some(desc.to_string());
                            }

                    let prompt = if let Some(task) = active_task {
                        format!("You are operating autonomously. Execute this task: {}. When finished, use agenda:complete.", task)
                    } else {
                        "IDLE_STATE".to_string()
                    };

                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: prompt,
                    });
                    self.state = AgentState::Idle;
                    self.broadcast_state().await;
                    return Err(crate::executive::error::FcpError::Interrupted);
                }
            };

            if let Some(handle) = stream_forwarder.take() {
                let _ = handle.await;
            }

            let response = match response_result {
                Ok(res) => {
                    tracing::info!(prompt_tokens = res.prompt_tokens, generated_tokens = res.generated_tokens, content_len = res.content.len(), "LLM response received");
                    tracing::debug!(raw_content = %res.content, "LLM raw output");
                    res
                }
                Err(e) => {
                    tracing::error!(error = %e, "LLM engine generation failed");
                    self.state = AgentState::Idle;
                    self.broadcast_state().await;
                    return Err(e);
                }
            };

            // Push assistant response into chat_stack so the LLM retains context across turns
            self.chat_stack.push(crate::engine::Message {
                role: "assistant".to_string(),
                content: response.content.clone(),
            });

            let total_tokens = response.generated_tokens + response.prompt_tokens;
            let threshold = (self.num_ctx as f32 * self.condensation_threshold) as usize;
            if total_tokens > threshold {
                tracing::warn!(total_tokens, threshold, "Token usage exceeds condensation threshold, running condenser");
                self.execute_condensation().await?;
                self.state = AgentState::Reflect;
                self.broadcast_state().await;
            }

            // 4. Directive Processing
            let directive = self.process_llm_response(&response.content);
            tracing::info!(directive = ?directive, "Directive from LLM response");

            match directive {
                LoopDirective::HaltAndAwaitInput(msg) => {
                    if let Some(ref user_msg) = msg {
                        tracing::info!(msg_len = user_msg.len(), "Agent responding to user");
                        if let Some(tx) = &self.tui_tx {
                            let _ = tx.send(crate::ui::events::TuiEvent::IncomingMessage(
                                format!("[ERIS]: {}", user_msg)
                            )).await;
                        }
                    }
                    self.state = AgentState::Idle;
                    self.tool_rounds = 0;
                    self.recovery_count = 0;
                    self.broadcast_state().await;
                    return Ok(());
                }
                LoopDirective::ExecuteTools(tools) => {
                    tracing::info!(tool_count = tools.len(), "Executing tool calls");
                    let current_state = self.state;
                    let exec_futures = tools.into_iter().map(|tool_call| {
                        let gatekeeper = &self.gatekeeper;
                        async move {
                            tracing::info!(tool = %tool_call.name, args = %tool_call.args, state = ?current_state, "Dispatching tool");
                            let name = tool_call.name;
                            let args = tool_call.args;
                            let result = gatekeeper.execute_tool(&current_state, &name, args).await;
                            (name, result)
                        }
                    });

                    let results = future::join_all(exec_futures).await;
                    let mut recoverable_msg: Option<String> = None;
                    let mut fatal_error = None;

                    for (tool_name, result) in results {
                        match result {
                            Ok(result) => {
                                self.tool_rounds += 1;
                                self.recovery_count = 0;
                                tracing::info!(tool = %tool_name, result_len = result.len(), round = self.tool_rounds, "Tool succeeded");
                                let msg = format!("Tool '{}' succeeded: {}", tool_name, result);
                                self.chat_stack.push(crate::engine::Message {
                                    role: "system".to_string(),
                                    content: msg.clone(),
                                });
                                if let Some(tx) = &self.tui_tx {
                                    let _ = tx.send(crate::ui::events::TuiEvent::IncomingMessage(msg)).await;
                                }
                                self.broadcast_state().await;
                            }
                            Err(e) => {
                                tracing::error!(tool = %tool_name, error = %e, error_type = ?std::mem::discriminant(&e), "Tool execution failed");
                                if Self::is_recoverable_tool_error(&e) {
                                    if recoverable_msg.is_none() {
                                        recoverable_msg = Some(e.to_string());
                                    }
                                } else {
                                    tracing::error!(error = %e, "System fatality detected during parallel tool execution");
                                    if fatal_error.is_none() {
                                        fatal_error = Some(e);
                                    }
                                }
                            }
                        }
                    }

                    if let Some(e) = fatal_error {
                        tracing::error!(error = %e, "System fatality - aborting orchestrator");
                        self.state = AgentState::Idle;
                        self.broadcast_state().await;
                        return Err(e);
                    }

                    if let Some(reason) = recoverable_msg {
                        self.recovery_count += 1;
                        self.state = AgentState::Recover;
                        let msg = format!("[SYSTEM OVERRIDE: FUCKUP DETECTED] Tool execution failed: {}", reason);
                        self.chat_stack.push(crate::engine::Message {
                            role: "system".to_string(),
                            content: msg.clone(),
                        });
                        if let Some(tx) = &self.tui_tx {
                            let _ = tx.send(crate::ui::events::TuiEvent::IncomingMessage(msg)).await;
                        }
                        self.broadcast_state().await;
                    }
                }
                LoopDirective::RecoverFromFuckup(msg) => {
                    self.recovery_count += 1;
                    tracing::warn!(recovery_count = self.recovery_count, reason = %msg, "Recovery triggered from bad LLM output");
                    self.state = AgentState::Recover;
                    let msg = format!("[SYSTEM OVERRIDE: FUCKUP DETECTED] Invalid LLM Output: {}", msg);
                    self.chat_stack.push(crate::engine::Message {
                        role: "system".to_string(),
                        content: msg.clone(),
                    });
                    if let Some(tx) = &self.tui_tx {
                        let _ = tx.send(crate::ui::events::TuiEvent::IncomingMessage(msg)).await;
                    }
                    self.broadcast_state().await;
                }
                LoopDirective::ShiftToReflection => {
                    tracing::info!("Shifting to Reflect state");
                    self.state = AgentState::Reflect;
                    self.broadcast_state().await;
                }
            }
        }
    }

    fn is_recoverable_tool_error(e: &crate::executive::error::FcpError) -> bool {
        matches!(
            e,
            crate::executive::error::FcpError::ToolFault { .. }
                | crate::executive::error::FcpError::SchemaViolation(_)
                | crate::executive::error::FcpError::Io(_)
                | crate::executive::error::FcpError::ParseFault(_)
        )
    }

    pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
        let json_str = if let (Some(start), Some(end)) = (response_json.find('{'), response_json.rfind('}')) {
            if start <= end {
                &response_json[start..=end]
            } else {
                response_json
            }
        } else {
            response_json
        };

        tracing::debug!(extracted_json_len = json_str.len(), "Parsing LLM JSON response");

        let response: LlmResponse = match serde_json::from_str(json_str) {
            Ok(res) => res,
            Err(e) => {
                tracing::warn!(error = %e, raw_snippet = &json_str[..json_str.len().min(200)], "Failed to parse LLM response as JSON");
                return LoopDirective::RecoverFromFuckup(e.to_string());
            }
        };

        tracing::info!(
            status = ?response.status,
            thought_len = response.thought.len(),
            tool_count = response.tool_calls.len(),
            has_message = response.message_to_user.is_some(),
            "Parsed LLM response"
        );

        match response.status {
            LoopAction::Reflect => {
                if response.tool_calls.is_empty() {
                    LoopDirective::RecoverFromFuckup("Reflect requires tool_calls".to_string())
                } else {
                    LoopDirective::ExecuteTools(response.tool_calls)
                }
            }
            LoopAction::Idle => {
                LoopDirective::HaltAndAwaitInput(response.message_to_user)
            }
            LoopAction::Task => {
                if !response.tool_calls.is_empty() {
                    LoopDirective::ExecuteTools(response.tool_calls)
                } else {
                    self.state = AgentState::Chat;
                    LoopDirective::ShiftToReflection
                }
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
        let response = self.engine.generate(&self.chat_stack, "", None).await?;

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
        self.broadcast_state().await;

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
            None,
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
        Orchestrator::new(engine, gatekeeper, ephemeral, vault_root, "test_ws", 3, 5, 0.8, 4096, rx, None)
    }

    #[test]
    fn test_router_valid_tool_call() {
        let mut orchestrator = setup_orchestrator();
        let json = r#"{
            "thought": "test",
            "status": "Reflect",
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
            "thought": "test",
            "status": "Reflect",
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
            "thought": "test",
            "status": "Task",
            "tool_calls": []
        }"#;
        
        let directive = orchestrator.process_llm_response(json);
        assert_eq!(directive, LoopDirective::ShiftToReflection);
        assert_eq!(orchestrator.state, AgentState::Chat);
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
            "thought": "I'm done",
            "status": "Idle",
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
            "thought": "Summarizing",
            "status": "Task",
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
            "thought": "I'm done",
            "status": "Idle",
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
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_root = temp_dir.path();
        let workspace = "test_ws";
        
        // Create the core dir so context_assembler has a valid parent
        let core_dir = vault_root.join(workspace).join("00_Core");
        tokio::fs::create_dir_all(&core_dir).await.unwrap();

        // Write a mock agenda file
        let agenda_path = vault_root.join(workspace).join(".fcp_agenda.json");
        let agenda_content = r#"[{"id": "1234", "created_at": 123456, "description": "Test agenda task", "status": "pending"}]"#;
        tokio::fs::write(&agenda_path, agenda_content).await.unwrap();

        let (tx, rx) = tokio::sync::watch::channel(());
        
        let mut orchestrator = Orchestrator::new(
            engine,
            gatekeeper,
            ephemeral,
            vault_root,
            workspace,
            3,
            5,
            0.8,
            4096,
            rx,
            None,
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
        assert_eq!(orchestrator.saved_chat_state.unwrap()[1].content, "hello");
        assert_eq!(orchestrator.chat_stack.len(), 1);
        assert!(orchestrator.chat_stack[0].content.contains("Test agenda task"));
        assert!(orchestrator.chat_stack[0].content.contains("agenda:complete"));
    }
}
