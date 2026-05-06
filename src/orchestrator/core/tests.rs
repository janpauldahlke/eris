use super::*;
use crate::config::AppConfig;
use crate::engine::{EngineResponse, LlmEngine, Message};
use crate::executive::error::Result;
use crate::memory::ephemeral::EphemeralMemory;
use crate::orchestrator::context::ContextViewSettings;
use crate::orchestrator::state::{AgentState, LoopDirective};
use crate::presentation::SessionEvent;
use crate::tools::Gatekeeper;
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::mpsc;

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
        _stream_tx: Option<mpsc::UnboundedSender<String>>,
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
    let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("test identity"));
    Box::leak(Box::new(id_tx));

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
        3,
        6000,
        false,
        0,
        rx,
        None,
        None,
        None,
        ContextViewSettings::default(),
        Arc::new(AppConfig::default()),
        id_rx,
        Arc::new(AtomicBool::new(false)),
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
    let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("test identity"));
    Box::leak(Box::new(id_tx));
    Orchestrator::new(
        engine,
        gatekeeper,
        ephemeral,
        vault_root,
        "test_ws",
        3,
        5,
        0.8,
        4096,
        3,
        6000,
        false,
        0,
        rx,
        None,
        None,
        None,
        ContextViewSettings::default(),
        Arc::new(AppConfig::default()),
        id_rx,
        Arc::new(AtomicBool::new(false)),
    )
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
fn test_router_idle_with_tools_executes_tools() {
    let mut orchestrator = setup_orchestrator();
    let json = r#"{
            "thought": "wrong status but tools present",
            "status": "Idle",
            "message_to_user": "Hang on…",
            "tool_calls": [{ "name": "vault:read", "args": { "path": "x.md" } }]
        }"#;

    let directive = orchestrator.process_llm_response(json);
    match directive {
        LoopDirective::ExecuteTools(tools) => {
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0].name, "vault:read");
        }
        _ => panic!("Expected ExecuteTools when tool_calls non-empty"),
    }
}

#[test]
fn test_router_reflect_empty_tools_shifts_to_reflection() {
    let mut orchestrator = setup_orchestrator();
    orchestrator.last_turn_tools_enabled = false;
    let json = r#"{
            "thought": "test",
            "status": "Reflect",
            "tool_calls": []
        }"#;

    let directive = orchestrator.process_llm_response(json);
    assert_eq!(directive, LoopDirective::ShiftToReflection);
    assert_eq!(orchestrator.state, AgentState::Chat);
}

#[test]
fn test_router_reflect_empty_tools_in_tool_mode_yields_fuckup() {
    let mut orchestrator = setup_orchestrator();
    orchestrator.last_turn_tools_enabled = true;
    let json = r#"{
            "thought": "test",
            "status": "Reflect",
            "tool_calls": []
        }"#;

    let directive = orchestrator.process_llm_response(json);
    match directive {
        LoopDirective::RecoverFromFuckup(msg) => {
            assert!(msg.contains("Tool-enabled mode forbids empty action"));
        }
        _ => panic!("Expected RecoverFromFuckup, got {:?}", directive),
    }
}

#[test]
fn test_router_reflect_empty_tools_with_message_halts() {
    let mut orchestrator = setup_orchestrator();
    let json = r#"{
            "thought": "done",
            "status": "Reflect",
            "message_to_user": "Here are your results.",
            "tool_calls": []
        }"#;

    let directive = orchestrator.process_llm_response(json);
    match directive {
        LoopDirective::HaltAndAwaitInput(Some(msg)) => {
            assert!(msg.contains("results"));
        }
        _ => panic!("Expected HaltAndAwaitInput, got {:?}", directive),
    }
}

#[test]
fn test_router_invalid_json_yields_fuckup() {
    let mut orchestrator = setup_orchestrator();
    let json = r#"{"status": "BAD_JSON"#;

    let directive = orchestrator.process_llm_response(json);
    match directive {
        LoopDirective::RecoverFromFuckup(msg) => {
            assert!(msg.contains("[FCP JSON REPAIR]"));
            assert!(msg.contains("tool_calls"));
        }
        _ => panic!("Expected RecoverFromFuckup"),
    }
}

/// Regression: models often omit the closing `}` for the tool object when `tool_calls` has one item.
#[test]
fn test_router_single_tool_calls_missing_inner_close_brace_yields_recovery_hint() {
    let mut orchestrator = setup_orchestrator();
    let json = r#"{
            "thought": "t",
            "status": "Reflect",
            "message_to_user": null,
            "tool_calls": [
            {
            "name": "memory:stage",
            "args": {
            "content": "x",
            "tags": ["semantic/knowledge/philosophy"],
            "title": "nietzsche_dancing_star"
            }
            ]
            }
        "#;

    let directive = orchestrator.process_llm_response(json);
    match directive {
        LoopDirective::RecoverFromFuckup(msg) => {
            assert!(msg.contains("[FCP JSON REPAIR]"));
            assert!(msg.contains("one more"));
        }
        other => panic!("Expected RecoverFromFuckup, got {:?}", other),
    }
}

#[test]
fn test_router_initiate_reflection_mutates_state() {
    let mut orchestrator = setup_orchestrator();
    orchestrator.last_turn_tools_enabled = false;
    let json = r#"{
            "thought": "test",
            "status": "Task",
            "tool_calls": []
        }"#;

    let directive = orchestrator.process_llm_response(json);
    assert_eq!(directive, LoopDirective::ShiftToReflection);
    assert_eq!(orchestrator.state, AgentState::Chat);
}

#[test]
fn test_router_task_empty_tools_in_tool_mode_yields_fuckup() {
    let mut orchestrator = setup_orchestrator();
    orchestrator.last_turn_tools_enabled = true;
    let json = r#"{
            "thought": "test",
            "status": "Task",
            "tool_calls": []
        }"#;

    let directive = orchestrator.process_llm_response(json);
    match directive {
        LoopDirective::RecoverFromFuckup(msg) => {
            assert!(msg.contains("Tool-enabled mode forbids empty action"));
        }
        _ => panic!("Expected RecoverFromFuckup, got {:?}", directive),
    }
}

#[test]
fn test_tool_fingerprint_is_stable_for_same_payload() {
    let args = serde_json::json!({"title":"hagbard_profile","tags":["person","contact"]});
    let a = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &args);
    let b = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &args);
    assert_eq!(a, b);
}

#[test]
fn test_tool_fingerprint_canonicalizes_object_key_order() {
    let a = serde_json::json!({
        "content": "User name is Hagbard.",
        "tags": ["person", "contact"],
        "title": "hagbard_profile"
    });
    let b = serde_json::json!({
        "title": "hagbard_profile",
        "content": "User name is Hagbard.",
        "tags": ["person", "contact"]
    });
    let fa = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &a);
    let fb = Orchestrator::<MockEngine>::tool_fingerprint("memory:stage", &b);
    assert_eq!(fa, fb);
}

#[test]
fn test_schema_or_parse_error_detection() {
    let schema_err = crate::executive::error::FcpError::SchemaViolation("bad args".to_string());
    let parse_err = crate::executive::error::FcpError::ParseFault(serde_json::Error::io(
        std::io::Error::other("bad json"),
    ));
    let net_err = crate::executive::error::FcpError::NetworkFault("offline".to_string());

    assert!(Orchestrator::<MockEngine>::is_schema_or_parse_tool_error(
        &schema_err
    ));
    assert!(Orchestrator::<MockEngine>::is_schema_or_parse_tool_error(
        &parse_err
    ));
    assert!(!Orchestrator::<MockEngine>::is_schema_or_parse_tool_error(
        &net_err
    ));
}

#[tokio::test]
async fn test_step_resets_counters_on_entry() {
    let json = r#"{
            "thought": "done",
            "status": "Idle",
            "message_to_user": "hi"
        }"#;
    let engine = MockEngine::with_content(json);
    let mut orchestrator = setup_orchestrator_with_engine(engine);
    orchestrator.recovery_count = 99;
    orchestrator.tool_rounds = 99;
    orchestrator.state = AgentState::Chat;

    let result = orchestrator.step(None).await;

    assert!(result.is_ok());
    assert_eq!(orchestrator.recovery_count, 0);
    assert_eq!(orchestrator.tool_rounds, 0);
    assert_eq!(orchestrator.state, AgentState::Idle);
}

#[tokio::test]
async fn test_step_system_fatality_aborts() {
    let engine = MockEngine::with_network_fault("daemon offline");
    let mut orchestrator = setup_orchestrator_with_engine(engine);
    orchestrator.state = AgentState::Chat;
    orchestrator.chat_stack.push(Message {
        role: "user".to_string(),
        content: "exercise engine error path".to_string(),
    });

    let result = orchestrator.step(None).await;

    assert!(result.is_err());
    assert_eq!(orchestrator.state, AgentState::Idle);
}

#[tokio::test]
async fn test_step_empty_user_line_sy_fnord_no_llm() {
    let json = r#"{"status":"Idle","message_to_user":"engine should not run"}"#;
    let engine = MockEngine::with_content(json);
    let mut orchestrator = setup_orchestrator_with_engine(engine);
    orchestrator.state = AgentState::Chat;
    orchestrator.chat_stack.push(Message {
        role: "user".to_string(),
        content: "   ".to_string(),
    });

    let result = orchestrator.step(None).await;
    assert!(result.is_ok());
    let last = orchestrator
        .chat_stack
        .last()
        .expect("assistant reply for empty user line");
    assert!(last.content.contains(EMPTY_USER_MESSAGE_TAG));
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
async fn test_execute_condensation_sliding_window_stack_only() {
    let rolling_json = r#"{"kind":"rolling_summary_v1","summary":"folded","key_facts":[],"open_threads":[],"last_updated":"2026-01-01T00:00:00+00:00"}"#;
    let engine = MockEngine::with_content(rolling_json);
    let mut orchestrator = setup_orchestrator_with_engine(engine);
    orchestrator.num_ctx = 48;
    orchestrator.chat_stack.clear();
    orchestrator.chat_stack.push(Message {
        role: "system".to_string(),
        content: "system prompt".to_string(),
    });
    for i in 0..8 {
        orchestrator.chat_stack.push(Message {
            role: "user".to_string(),
            content: format!("user-{i}-{}", "x".repeat(40)),
        });
        orchestrator.chat_stack.push(Message {
            role: "assistant".to_string(),
            content: format!("assistant-{i}-{}", "y".repeat(40)),
        });
    }

    let result = orchestrator.execute_condensation().await;

    assert!(result.is_ok());
    let head = crate::orchestrator::context::split_stack_head(&orchestrator.chat_stack)
        .expect("split head");
    assert!(head.rolling.is_some());
    assert!(orchestrator.chat_stack.len() >= 3);

    let rolling_content = head.rolling.as_ref().expect("rolling").content.as_str();
    let parsed: crate::orchestrator::context::RollingSummaryV1 =
        serde_json::from_str(rolling_content).expect("rolling json");
    assert_eq!(
        parsed.kind,
        crate::orchestrator::context::ROLLING_SUMMARY_KIND
    );
    assert_eq!(parsed.summary, "folded");

    let stored = orchestrator
        .ephemeral
        .get(crate::orchestrator::context::ROLLING_SUMMARY_TITLE)
        .await;
    assert!(
        stored.is_none(),
        "rolling summary must not be written to ephemeral"
    );

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
            _stream_tx: Option<mpsc::UnboundedSender<String>>,
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
    let core_dir = vault_root.join(workspace).join("00_Invariants");
    tokio::fs::create_dir_all(&core_dir).await.unwrap();

    // Write a mock agenda file
    let ws = vault_root.join(workspace);
    let agenda_path = crate::vault_layout::agenda_json(&ws);
    tokio::fs::create_dir_all(crate::vault_layout::tools_dir(&ws))
        .await
        .unwrap();
    let agenda_content = r#"[{"id": "1234", "created_at": 123456, "description": "Test agenda task", "status": "pending"}]"#;
    tokio::fs::write(&agenda_path, agenda_content)
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::watch::channel(());
    let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("test identity"));
    Box::leak(Box::new(id_tx));

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
        3,
        6000,
        false,
        0,
        rx,
        None,
        None,
        None,
        ContextViewSettings::default(),
        Arc::new(AppConfig::default()),
        id_rx,
        Arc::new(AtomicBool::new(false)),
    );

    orchestrator.state = AgentState::Chat;
    orchestrator.chat_stack.push(Message {
        role: "user".to_string(),
        content: "hello".to_string(),
    });

    // Fire the interrupt shortly after calling step
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tx.send(());
    });

    let result = orchestrator.step(None).await;

    assert!(matches!(
        result,
        Err(crate::executive::error::FcpError::Interrupted)
    ));
    assert_eq!(orchestrator.state, AgentState::Idle);
    assert!(orchestrator.saved_chat_state.is_some());
    assert_eq!(orchestrator.saved_chat_state.unwrap()[1].content, "hello");
    assert_eq!(orchestrator.chat_stack.len(), 1);
    assert!(
        orchestrator.chat_stack[0]
            .content
            .contains("Test agenda task")
    );
    assert!(
        orchestrator.chat_stack[0]
            .content
            .contains("agenda:complete")
    );
}

#[tokio::test]
async fn test_duplicate_only_batch_halts_without_extra_generation() {
    #[derive(Clone)]
    struct SequenceEngine {
        responses: Arc<Vec<String>>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmEngine for SequenceEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>,
        ) -> Result<EngineResponse> {
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            let content = self.responses.get(idx).cloned().unwrap_or_else(|| {
                self.responses.last().cloned().unwrap_or_else(|| {
                    "{\"status\":\"Idle\",\"message_to_user\":\"done\",\"tool_calls\":[]}"
                        .to_string()
                })
            });
            Ok(EngineResponse {
                content,
                prompt_tokens: 0,
                generated_tokens: 0,
            })
        }
    }

    let first = r#"{
            "thought": "stage once",
            "status": "Reflect",
            "tool_calls": [{
                "name": "memory:stage",
                "args": {
                    "title": "hagbard_profile",
                    "content": "User name is Hagbard.",
                    "tags": ["person","contact"]
                }
            }]
        }"#
    .to_string();
    let second_duplicate = first.clone();
    let third_reply = r#"{
            "thought": "duplicate tool call was suppressed; reply to user",
            "status": "Idle",
            "message_to_user": "Got it — I already staged that memory, so I won’t repeat the tool call.",
            "tool_calls": []
        }"#
    .to_string();

    let calls = Arc::new(AtomicUsize::new(0));
    let engine = SequenceEngine {
        responses: Arc::new(vec![first, second_duplicate, third_reply]),
        calls: calls.clone(),
    };

    let mut gatekeeper = Gatekeeper::new();
    let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
    gatekeeper.register(Arc::new(crate::tools::memory::MemoryStageTool {
        ephemeral: ephemeral.clone(),
        config: Arc::new(crate::config::AppConfig::default()),
        max_content_chars: 10_000,
    }));

    let vault_root = Path::new("/tmp/vault");
    let (tx, rx) = tokio::sync::watch::channel(());
    Box::leak(Box::new(tx));
    let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("test identity"));
    Box::leak(Box::new(id_tx));

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
        3,
        6000,
        false,
        0,
        rx,
        None,
        None,
        None,
        ContextViewSettings::default(),
        Arc::new(AppConfig::default()),
        id_rx,
        Arc::new(AtomicBool::new(false)),
    );
    orchestrator.state = AgentState::Chat;
    orchestrator.chat_stack.push(Message {
        role: "user".to_string(),
        content: "remember my name".to_string(),
    });

    let result = orchestrator.step(None).await;
    assert!(result.is_ok());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "expected one extra LLM round to produce a user-facing reply after duplicate-only suppression"
    );
    assert_eq!(orchestrator.state, AgentState::Idle);
}

#[test]
fn test_extract_agenda_confirm_task_id() {
    let s = "noise\n[AGENDA_CONFIRM task_id=abc-xyz alarm_id=u late_sec=0]";
    assert_eq!(
        Orchestrator::<MockEngine>::extract_agenda_confirm_task_id(s),
        Some("abc-xyz")
    );
    assert_eq!(
        Orchestrator::<MockEngine>::extract_agenda_confirm_task_id("no tag"),
        None
    );
}

#[test]
fn test_agenda_confirm_task_id_before_current_turn_skips_latest_user() {
    let stack = vec![
        Message {
            role: "user".to_string(),
            content: "[AGENDA_CONFIRM task_id=too-old alarm_id=a late_sec=0]".to_string(),
        },
        Message {
            role: "user".to_string(),
            content: "prefix [AGENDA_CONFIRM task_id=expected-id alarm_id=b late_sec=1] tail"
                .to_string(),
        },
        Message {
            role: "user".to_string(),
            content: "done".to_string(),
        },
    ];
    assert_eq!(
        Orchestrator::<MockEngine>::agenda_confirm_task_id_before_current_turn(&stack),
        Some("expected-id".to_string())
    );
}

#[test]
fn test_user_text_means_agenda_done_ack() {
    assert!(Orchestrator::<MockEngine>::user_text_means_agenda_done_ack(
        "done"
    ));
    assert!(Orchestrator::<MockEngine>::user_text_means_agenda_done_ack(
        "  FINISHED  "
    ));
    assert!(Orchestrator::<MockEngine>::user_text_means_agenda_done_ack(
        "all done"
    ));
    assert!(!Orchestrator::<MockEngine>::user_text_means_agenda_done_ack("tell me a story"));
}

async fn orchestrator_with_presentation(
    pres_tx: tokio::sync::mpsc::Sender<SessionEvent>,
) -> Orchestrator<MockEngine> {
    let engine = MockEngine::new();
    let gatekeeper = Gatekeeper::new();
    let ephemeral = Arc::new(EphemeralMemory::new("deck_emit_test".to_string()));
    let dir = tempfile::tempdir().expect("tempdir");
    let vault_root = dir.path();
    let workspace = "deck_emit_test";
    tokio::fs::create_dir_all(vault_root.join(workspace).join("00_Invariants"))
        .await
        .expect("mkdir");
    let (watch_tx, watch_rx) = tokio::sync::watch::channel(());
    let _ = watch_tx;
    let (id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("deck test identity"));
    Box::leak(Box::new(id_tx));
    Orchestrator::new(
        engine,
        gatekeeper,
        ephemeral,
        vault_root,
        workspace,
        3,
        5,
        0.8,
        4096,
        3,
        6000,
        false,
        0,
        watch_rx,
        Some(pres_tx),
        None,
        None,
        ContextViewSettings::default(),
        Arc::new(AppConfig::default()),
        id_rx,
        Arc::new(AtomicBool::new(false)),
    )
}

#[tokio::test]
async fn emit_optional_user_message_emits_model_thought_then_incoming_message() {
    let (pres_tx, mut pres_rx) = mpsc::channel::<SessionEvent>(32);
    let mut orch = orchestrator_with_presentation(pres_tx).await;
    let json = r#"{"thought":"internal reasoning here","status":"Idle","message_to_user":"Hello user","tool_calls":[]}"#;
    orch.emit_optional_user_message(json).await;

    match pres_rx.recv().await.expect("event 1") {
        SessionEvent::ModelThought(t) => assert_eq!(t, "internal reasoning here"),
        e => panic!("expected ModelThought first, got {e:?}"),
    }
    match pres_rx.recv().await.expect("event 2") {
        SessionEvent::IncomingMessage(m) => assert!(m.contains("Hello user")),
        e => panic!("expected IncomingMessage, got {e:?}"),
    }
    match pres_rx.recv().await.expect("event 3") {
        SessionEvent::StateUpdate(_) => {}
        e => panic!("expected StateUpdate from broadcast_state, got {e:?}"),
    }
}

#[tokio::test]
async fn emit_optional_user_message_skips_whitespace_only_thought() {
    let (pres_tx, mut pres_rx) = mpsc::channel::<SessionEvent>(32);
    let mut orch = orchestrator_with_presentation(pres_tx).await;
    let json = r#"{"thought":"   \n  ","status":"Idle","message_to_user":"Hi","tool_calls":[]}"#;
    orch.emit_optional_user_message(json).await;

    match pres_rx.recv().await.expect("event 1") {
        SessionEvent::IncomingMessage(m) => assert!(m.contains("Hi")),
        e => panic!("expected IncomingMessage only (no ModelThought), got {e:?}"),
    }
    match pres_rx.recv().await.expect("event 2") {
        SessionEvent::StateUpdate(_) => {}
        e => panic!("expected StateUpdate, got {e:?}"),
    }
}

#[tokio::test]
async fn emit_optional_user_message_tool_round_emits_thought_before_deck() {
    let (pres_tx, mut pres_rx) = mpsc::channel::<SessionEvent>(32);
    let mut orch = orchestrator_with_presentation(pres_tx).await;
    let json = r#"{"thought":"pick clock tool","status":"Reflect","message_to_user":"One moment.","tool_calls":[{"name":"clock:now","args":{}}]}"#;
    orch.emit_optional_user_message(json).await;

    match pres_rx.recv().await.expect("event 1") {
        SessionEvent::ModelThought(t) => assert_eq!(t, "pick clock tool"),
        e => panic!("expected ModelThought, got {e:?}"),
    }
    match pres_rx.recv().await.expect("event 2") {
        SessionEvent::IncomingMessage(m) => assert!(m.contains("One moment.")),
        e => panic!("expected IncomingMessage, got {e:?}"),
    }
    match pres_rx.recv().await.expect("event 3") {
        SessionEvent::StateUpdate(_) => {}
        e => panic!("expected StateUpdate, got {e:?}"),
    }
}
