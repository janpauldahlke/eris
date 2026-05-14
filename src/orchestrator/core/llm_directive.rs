use crate::engine::LlmEngine;
use crate::orchestrator::llm_support::json_envelope::{
    llm_json_parse_recovery_message_with_excerpt, parse_llm_response_protocol,
    strip_leading_redacted_thinking_block,
};
use crate::orchestrator::state::{AgentState, LlmResponse, LoopAction, LoopDirective};

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    /// Maps JSON parse failure to recovery. **Ollama:** legacy excerpt-only message. **llama.cpp:**
    /// GBNF-oriented warning + `[PROTOCOL_JSON]`-prefixed recovery.
    pub(super) fn protocol_parse_failure_directive(
        &self,
        err: &serde_json::Error,
        raw: &str,
    ) -> LoopDirective {
        if self.config.is_llamacpp() {
            tracing::warn!(
                error = %err,
                "llama.cpp: model output was not valid FCP protocol JSON while GBNF was active (grammar constrains sampling but does not guarantee JSON; common causes: Qwen `<think>` / template prose before `{{`, truncation, duplicate JSON blobs, or server-side grammar not applied under very large prompts)"
            );
            LoopDirective::RecoverFromFuckup(format!(
                "[PROTOCOL_JSON] {}",
                llm_json_parse_recovery_message_with_excerpt(err, raw)
            ))
        } else {
            LoopDirective::RecoverFromFuckup(llm_json_parse_recovery_message_with_excerpt(
                err, raw,
            ))
        }
    }

    /// Parses FCP JSON from the model. On failure, **llama.cpp** may retry after stripping a leading
    /// `redacted_thinking` block, then uses [`Self::protocol_parse_failure_directive`] (GBNF-oriented
    /// log + `[PROTOCOL_JSON]` recovery). **Ollama** never strips that wrapper and uses the legacy
    /// recovery message only (no `[PROTOCOL_JSON]` prefix on parse failure).
    pub fn process_llm_response(&mut self, response_json: &str) -> LoopDirective {
        match parse_llm_response_protocol(response_json) {
            Ok(parsed) => self.directive_from_parsed(parsed),
            Err(e) => {
                if self.config.is_llamacpp() {
                    let stripped = strip_leading_redacted_thinking_block(response_json);
                    if stripped != response_json {
                        if let Ok(parsed) = parse_llm_response_protocol(stripped) {
                            tracing::info!(
                                event = "strip_redacted_thinking_ok",
                                "Parsed FCP protocol JSON after stripping leading redacted_thinking block"
                            );
                            return self.directive_from_parsed(parsed);
                        }
                    }
                }
                self.protocol_parse_failure_directive(&e, response_json)
            }
        }
    }

    /// Directive path for an already-parsed [`LlmResponse`] (avoids a second parse after `step` preflight).
    pub(super) fn directive_from_parsed(&mut self, parsed: LlmResponse) -> LoopDirective {
        let explicit_status = parsed.has_explicit_status();
        let status = parsed.status();
        tracing::info!(
            status = ?status,
            explicit_status,
            thought_len = parsed.thought.len(),
            tool_count = parsed.tool_calls.len(),
            has_message = parsed.message_to_user.is_some(),
            "Parsed LLM response"
        );

        if !explicit_status
            && parsed.tool_calls.is_empty()
            && parsed
                .message_to_user
                .as_ref()
                .is_none_or(|m| m.trim().is_empty())
        {
            let msg = if self.config.is_llamacpp() {
                "Missing required `status` and no tool_calls or message_to_user.".to_string()
            } else {
                "Missing required `status` and no actionable fields (`tool_calls`/`message_to_user`)"
                    .to_string()
            };
            return LoopDirective::RecoverFromFuckup(msg);
        }

        if !parsed.tool_calls.is_empty() {
            return LoopDirective::ExecuteTools(parsed.tool_calls);
        }

        let tool_mode_empty_action = self.last_turn_tools_enabled
            && parsed.tool_calls.is_empty()
            && parsed
                .message_to_user
                .as_ref()
                .is_none_or(|m| m.trim().is_empty());

        match status {
            LoopAction::Reflect => {
                if let Some(msg) = parsed.message_to_user
                    && !msg.trim().is_empty()
                {
                    return LoopDirective::HaltAndAwaitInput(Some(msg));
                }
                if tool_mode_empty_action {
                    let msg = if self.config.is_llamacpp() {
                        "Empty action: include tool_calls or a non-empty message_to_user.".to_string()
                    } else {
                        "Tool-enabled mode forbids empty action: status Reflect with empty tool_calls and empty message_to_user. Use Reflect with tool_calls, or Idle with non-empty message_to_user.".to_string()
                    };
                    return LoopDirective::RecoverFromFuckup(msg);
                }
                tracing::debug!("Reflect with empty tool_calls — treating as Task");
                self.state = AgentState::Chat;
                LoopDirective::ShiftToReflection
            }
            LoopAction::Idle => match parsed.message_to_user {
                Some(msg) if !msg.trim().is_empty() => LoopDirective::HaltAndAwaitInput(Some(msg)),
                _ => {
                    let thought = parsed.thought.trim();
                    if !thought.is_empty() {
                        LoopDirective::HaltAndAwaitInput(Some(thought.to_string()))
                    } else {
                        let msg = if self.config.is_llamacpp() {
                            "Idle requires a non-empty message_to_user (or non-empty thought)."
                                .to_string()
                        } else {
                            "Idle status requires non-empty message_to_user (or non-empty thought as fallback)".to_string()
                        };
                        LoopDirective::RecoverFromFuckup(msg)
                    }
                }
            },
            LoopAction::Task => {
                if tool_mode_empty_action {
                    let msg = if self.config.is_llamacpp() {
                        "Empty action: include tool_calls or a non-empty message_to_user.".to_string()
                    } else {
                        "Tool-enabled mode forbids empty action: status Task with empty tool_calls and empty message_to_user. Use Reflect with tool_calls, or Idle with non-empty message_to_user.".to_string()
                    };
                    return LoopDirective::RecoverFromFuckup(msg);
                }
                self.state = AgentState::Chat;
                LoopDirective::ShiftToReflection
            }
        }
    }
}

#[cfg(test)]
mod phase5_recovery_tests {
    use super::*;
    use crate::config::{AppConfig, LlmBackend};
    use crate::engine::{EngineResponse, LlmEngine, LlmGenerateOptions, Message};
    use crate::memory::ephemeral::EphemeralMemory;
    use crate::orchestrator::context::ContextViewSettings;
    use crate::orchestrator::llm_support::json_envelope::FCP_JSON_REPAIR_MARKER;
    use crate::tools::Gatekeeper;
    use async_trait::async_trait;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::watch;
    use tracing_test::traced_test;

    struct StubEngine;

    #[async_trait]
    impl LlmEngine for StubEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
            _options: LlmGenerateOptions,
        ) -> crate::executive::error::Result<EngineResponse> {
            Ok(EngineResponse {
                content: "{}".into(),
                prompt_tokens: 0,
                generated_tokens: 0,
                generation_ms: 0,
            })
        }
    }

    fn orchestrator_with_config(config: AppConfig) -> Orchestrator<StubEngine> {
        let gatekeeper = Gatekeeper::new();
        let ephemeral = Arc::new(EphemeralMemory::new("test_ws".to_string()));
        let vault_root = Path::new("/tmp/vault");
        let (tx, rx) = watch::channel(());
        Box::leak(Box::new(tx));
        let (id_tx, id_rx) = watch::channel(Arc::from("id"));
        Box::leak(Box::new(id_tx));
        Orchestrator::new(
            StubEngine,
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
            Arc::new(config),
            id_rx,
            Arc::new(AtomicBool::new(false)),
            None,
        )
    }

    #[traced_test]
    #[test]
    fn grammar_path_json_parse_failure_logs_llama_protocol() {
        let mut cfg = AppConfig::default();
        cfg.llm_backend = LlmBackend::LlamaCpp;
        let mut orch = orchestrator_with_config(cfg);
        let directive = orch.process_llm_response(r#"{"broken""#);
        assert!(
            logs_contain("llama.cpp: model output was not valid FCP protocol JSON"),
            "expected warn log for llama.cpp protocol parse failure"
        );
        match directive {
            LoopDirective::RecoverFromFuckup(msg) => {
                assert!(msg.contains("[PROTOCOL_JSON]"));
                assert!(msg.contains(FCP_JSON_REPAIR_MARKER));
            }
            other => panic!("expected RecoverFromFuckup, got {:?}", other),
        }
    }

    #[test]
    fn grammar_path_semantic_violation_short_message() {
        let mut cfg = AppConfig::default();
        cfg.llm_backend = LlmBackend::LlamaCpp;
        let mut orch = orchestrator_with_config(cfg);
        orch.last_turn_tools_enabled = true;
        let json = r#"{
            "thought": "test",
            "status": "Task",
            "tool_calls": []
        }"#;
        let directive = orch.process_llm_response(json);
        let short = match directive {
            LoopDirective::RecoverFromFuckup(m) => m,
            other => panic!("unexpected {:?}", other),
        };
        let mut ollama_orch = orchestrator_with_config(AppConfig::default());
        ollama_orch.last_turn_tools_enabled = true;
        let long = match ollama_orch.process_llm_response(json) {
            LoopDirective::RecoverFromFuckup(m) => m,
            other => panic!("unexpected {:?}", other),
        };
        assert!(short.len() < long.len());
        assert_eq!(
            short,
            "Empty action: include tool_calls or a non-empty message_to_user."
        );
    }

    #[test]
    fn ollama_path_recovery_unchanged() {
        let mut orch = orchestrator_with_config(AppConfig::default());
        let json = r#"{"status": "BAD_JSON"#;
        let directive = orch.process_llm_response(json);
        match directive {
            LoopDirective::RecoverFromFuckup(msg) => {
                assert!(!msg.contains("[PROTOCOL_JSON]"));
                assert!(msg.contains(FCP_JSON_REPAIR_MARKER));
                assert!(msg.contains("tool_calls"));
                assert!(msg.contains("one more"));
            }
            other => panic!("unexpected {:?}", other),
        }
    }
}
