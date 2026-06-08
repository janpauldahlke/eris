use crate::engine::LlmEngine;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;
use serde_json::json;
use std::collections::HashSet;
use std::time::Instant;

use super::{EMPTY_USER_MESSAGE_TAG, Orchestrator};

const EMPTY_USER_SHRUGS: &[&str] = &["¯\\_(ツ)_/¯", "(・_・)", "(╯°□°）╯︵ ┻━┻"];

impl<E: LlmEngine> Orchestrator<E> {
    /// If the user clearly finished an agenda-linked alarm task, complete it without an LLM round trip.
    pub(super) async fn maybe_run_deterministic_agenda_complete(
        &mut self,
        step_start: Instant,
    ) -> Result<bool> {
        let user_line = self.last_user_content();
        if !Self::user_text_means_agenda_done_ack(user_line) {
            return Ok(false);
        }
        let Some(task_id) = Self::agenda_confirm_task_id_before_current_turn(&self.chat_stack)
        else {
            return Ok(false);
        };

        tracing::info!(
            task_id = %task_id,
            event = "orchestrator.agenda.deterministic_complete",
            "Running agenda:complete from explicit done after AGENDA_CONFIRM"
        );

        let tool_started = Instant::now();
        let args = json!({
            "task_id": task_id,
            "result_summary": "User confirmed completion (deterministic path after agenda alarm)."
        });
        let result = self
            .gatekeeper
            .execute_tool(&AgentState::Idle, "agenda:complete", args)
            .await;
        let tool_ms = tool_started.elapsed().as_millis() as u64;

        match result {
            Ok(tool_out) => {
                let preview = tool_out.chars().take(200).collect::<String>();
                tracing::info!(
                    tool_ms,
                    preview = %preview,
                    event = "orchestrator.agenda.deterministic_complete_ok",
                    "agenda:complete succeeded"
                );
                let deck_msg = format!(
                    "Marked that agenda task as done. {}",
                    tool_out.chars().take(120).collect::<String>()
                );
                let content = serde_json::to_string(&json!({
                    "thought": "User confirmed task completion; agenda:complete executed deterministically.",
                    "status": "Idle",
                    "message_to_user": deck_msg,
                    "tool_calls": []
                }))
                .map_err(|e| FcpError::EngineFault(e.to_string()))?;

                self.emit_optional_user_message(&content).await;
                self.chat_stack.push(crate::engine::Message {
                    role: "assistant".to_string(),
                    content,
                });
                self.state = AgentState::Idle;
                self.recovery_count = 0;
                self.tool_rounds = 0;
                self.last_llm_ms = 0;
                self.last_tool_ms = tool_ms;
                self.last_total_ms = step_start.elapsed().as_millis() as u64;
                self.last_turn_tools_enabled = false;
                self.broadcast_state().await;
                Ok(true)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    task_id = %task_id,
                    "Deterministic agenda:complete failed; continuing with normal LLM step"
                );
                Ok(false)
            }
        }
    }

    pub(super) async fn handle_empty_user_turn(&mut self) -> Result<()> {
        let idx = self.chat_stack.len() % EMPTY_USER_SHRUGS.len().max(1);
        let face = EMPTY_USER_SHRUGS[idx];
        let thought = format!("{} — empty last user message", EMPTY_USER_MESSAGE_TAG);
        let message_to_user = format!("{face} {}", EMPTY_USER_MESSAGE_TAG);
        let value = serde_json::json!({
            "thought": thought,
            "status": "Idle",
            "message_to_user": message_to_user,
            "tool_calls": []
        });
        let content = serde_json::to_string(&value)?;
        self.emit_optional_user_message(&content).await;
        self.chat_stack.push(crate::engine::Message {
            role: "assistant".to_string(),
            content,
        });
        self.state = AgentState::Idle;
        self.last_llm_ms = 0;
        self.last_total_ms = 0;
        self.broadcast_state().await;
        Ok(())
    }

    pub(super) fn build_descriptor_jit_guidance(
        &self,
        state: &AgentState,
        router_matches: &[String],
        targeted_tools: &HashSet<String>,
    ) -> Option<String> {
        let registry = self.descriptor_registry.as_ref()?;
        let mut selected = if !targeted_tools.is_empty() {
            targeted_tools.iter().cloned().collect::<Vec<_>>()
        } else {
            router_matches
                .iter()
                .take(self.descriptor_jit_top_k.max(1))
                .cloned()
                .collect::<Vec<_>>()
        };
        if selected.is_empty() {
            return None;
        }
        selected.sort();
        selected.dedup();

        let allowed_names = self
            .gatekeeper
            .get_allowed_tools(state)
            .into_iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<HashSet<_>>();

        let mut sections = Vec::new();
        let mut used = 0usize;
        let max_chars = self.descriptor_jit_max_chars.max(500);
        for name in selected {
            if !allowed_names.contains(&name) {
                continue;
            }
            let Some(desc) = registry.get(&name) else {
                continue;
            };
            let snippet = format!(
                "Tool: {}\nWhen to use: {}\nWhen not to use: {}\nGood examples: {}\nBad examples: {}",
                desc.tool_name,
                desc.when_to_use.as_deref().unwrap_or("n/a"),
                desc.when_not_to_use.as_deref().unwrap_or("n/a"),
                desc.examples_good
                    .iter()
                    .take(2)
                    .map(|e| format!("{} {}", e.name, e.args))
                    .collect::<Vec<_>>()
                    .join(" | "),
                desc.examples_bad
                    .iter()
                    .take(2)
                    .map(|e| format!("{} {}", e.name, e.args))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            if used + snippet.len() > max_chars {
                break;
            }
            used += snippet.len();
            sections.push(snippet);
        }
        if sections.is_empty() {
            return None;
        }
        tracing::debug!(
            jit_section_chars = used,
            jit_section_cap = max_chars,
            selected_tools = sections.len(),
            "Descriptor JIT guidance budget usage"
        );
        Some(format!(
            "[JIT TOOL GUIDANCE]\nUse the following targeted tool guidance while keeping args fully compliant with provided JSON schemas.\n{}\n[/JIT TOOL GUIDANCE]",
            sections.join("\n\n")
        ))
    }

    /// Narrow the next Recover hop to full parameter schemas for the tool(s) that failed.
    ///
    /// Schema-fault retry already sets [`Orchestrator::force_full_tool_schemas_in_llm_view`] and
    /// `targeted_tools`; protocol JSON parse and recoverable tool failures use this helper.
    pub(super) fn arm_recover_pass_with_targeted_full_schemas(
        &mut self,
        targeted_tools: &mut HashSet<String>,
        raw_llm_output: Option<&str>,
        router_hints: &[String],
    ) {
        if self.force_full_tool_schemas_in_llm_view && !targeted_tools.is_empty() {
            return;
        }
        let allowed: HashSet<String> = self
            .gatekeeper
            .get_allowed_tools(&AgentState::Chat)
            .into_iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        let step_failed: Vec<String> = self.step_failed_tools.iter().cloned().collect();
        let candidates = crate::orchestrator::llm_support::json_envelope::select_recovery_targeted_tools(
            raw_llm_output,
            self.last_user_content(),
            &step_failed,
            router_hints,
            &self.chat_stack,
            &allowed,
            self.tool_map_offer_cap,
        );
        if candidates.is_empty() {
            return;
        }
        targeted_tools.clear();
        for name in candidates {
            targeted_tools.insert(name);
        }
        crate::orchestrator::llm_support::post_tool_guidance::ensure_web_find_paired_with_fetch_tools(
            targeted_tools,
            &allowed,
        );
        crate::orchestrator::llm_support::post_tool_guidance::expand_web_tools_for_protocol_recover(
            targeted_tools,
            &allowed,
        );
        self.force_full_tool_schemas_in_llm_view = true;
        tracing::info!(
            targeted_tools = ?targeted_tools,
            "Recover pass armed with targeted full tool schemas"
        );
    }

    pub(super) async fn build_skill_jit_guidance(
        &self,
        state: &AgentState,
        router_matches: &[String],
        targeted_tools: &HashSet<String>,
        failed_tools: &HashSet<String>,
    ) -> Result<Option<String>> {
        let Some(registry) = self.descriptor_registry.as_ref() else {
            return Ok(None);
        };
        let allowed_names = self
            .gatekeeper
            .get_allowed_tools(state)
            .into_iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<HashSet<_>>();
        let mut candidate_tools = HashSet::<String>::new();
        for name in router_matches {
            if allowed_names.contains(name) {
                candidate_tools.insert(name.clone());
            }
        }
        for name in targeted_tools {
            if allowed_names.contains(name) {
                candidate_tools.insert(name.clone());
            }
        }
        for name in failed_tools {
            if allowed_names.contains(name) {
                candidate_tools.insert(name.clone());
            }
        }
        let mut selected_skill_ids = Vec::<String>::new();
        if candidate_tools.iter().any(|n| n == "mail:write") {
            selected_skill_ids.push("mail-recipient-verify".to_string());
        }
        for tool_name in candidate_tools {
            if let Some(desc) = registry.get(&tool_name) {
                for skill in &desc.suggested_skills {
                    selected_skill_ids.push(skill.clone());
                }
            }
        }
        selected_skill_ids.retain(|id| is_operational_skill_id(id));
        if selected_skill_ids.is_empty() {
            return Ok(None);
        }
        let Some(workspace_root) = self.context_assembler.core_dir.parent() else {
            return Ok(None);
        };
        let max_chars = (self.descriptor_jit_max_chars / 2).max(500);
        crate::skills::build_jit_skill_guidance(workspace_root, &selected_skill_ids, max_chars).await
    }
}

fn is_operational_skill_id(id: &str) -> bool {
    !matches!(id, "skill-authoring-meta")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::engine::{EngineResponse, LlmEngine, LlmGenerateOptions, Message};
    use crate::memory::ephemeral::EphemeralMemory;
    use crate::orchestrator::context::ContextViewSettings;
    use crate::tools::Gatekeeper;
    use crate::tools::traits::Tool;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::mpsc;

    struct StubEngine;

    #[async_trait]
    impl LlmEngine for StubEngine {
        async fn generate(
            &self,
            _stack: &[Message],
            _available_tools_json: &str,
            _stream_tx: Option<mpsc::UnboundedSender<String>>,
            _options: LlmGenerateOptions,
        ) -> Result<EngineResponse> {
            Ok(EngineResponse {
                content: "{}".into(),
                prompt_tokens: 0,
                generated_tokens: 0,
                generation_ms: 0,
            })
        }
    }

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    struct MailWriteProbeTool;
    struct DbFindProbeTool;

    #[async_trait]
    impl Tool for MailWriteProbeTool {
        fn name(&self) -> &'static str {
            "mail:write"
        }
        fn description(&self) -> &'static str {
            "probe"
        }
        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schemars::schema_for!(EmptyArgs)
        }
        async fn execute(&self, _args: serde_json::Value) -> Result<String> {
            Ok("ok".to_string())
        }
    }

    #[async_trait]
    impl Tool for DbFindProbeTool {
        fn name(&self) -> &'static str {
            "db:find_connections"
        }
        fn description(&self) -> &'static str {
            "probe"
        }
        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schemars::schema_for!(EmptyArgs)
        }
        async fn execute(&self, _args: serde_json::Value) -> Result<String> {
            Ok("ok".to_string())
        }
    }

    async fn test_orchestrator_with_skills() -> (Orchestrator<StubEngine>, tempfile::TempDir) {
        let root = tempfile::tempdir().expect("tempdir");
        let workspace_root = root.path().join("ws");
        tokio::fs::create_dir_all(&workspace_root)
            .await
            .expect("workspace create");
        crate::skills::seed_runtime_skills(&workspace_root)
            .await
            .expect("seed skills");

        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(MailWriteProbeTool));
        gatekeeper.register(Arc::new(DbFindProbeTool));

        let (_tx, interrupt_rx) = tokio::sync::watch::channel(());
        let (_id_tx, id_rx) = tokio::sync::watch::channel(Arc::from("identity"));

        let orch = Orchestrator::new(
            StubEngine,
            gatekeeper,
            Arc::new(EphemeralMemory::new("ws".to_string())),
            root.path(),
            "ws",
            3,
            5,
            0.8,
            4096,
            3,
            4000,
            false,
            0,
            interrupt_rx,
            None,
            None,
            Some(Arc::new(
                crate::tools::ToolDescriptorRegistry::load_embedded()
                    .expect("descriptor registry"),
            )),
            ContextViewSettings::default(),
            Arc::new(AppConfig::default()),
            id_rx,
            Arc::new(AtomicBool::new(false)),
            None,
            None,
        );
        (orch, root)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skill_guidance_none_when_no_candidates() {
        let (orch, _root) = test_orchestrator_with_skills().await;
        let none = orch
            .build_skill_jit_guidance(
                &AgentState::Chat,
                &[],
                &HashSet::new(),
                &HashSet::new(),
            )
            .await
            .expect("skill guidance");
        assert!(none.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skill_guidance_includes_failed_db_recovery() {
        let (orch, _root) = test_orchestrator_with_skills().await;
        let mut failed = HashSet::new();
        failed.insert("db:find_connections".to_string());
        let out = orch
            .build_skill_jit_guidance(&AgentState::Chat, &[], &HashSet::new(), &failed)
            .await
            .expect("skill guidance")
            .expect("some");
        assert!(out.contains("db-connections-recovery"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skill_guidance_includes_mail_mandatory_on_router_hit() {
        let (orch, _root) = test_orchestrator_with_skills().await;
        let out = orch
            .build_skill_jit_guidance(
                &AgentState::Chat,
                &["mail:write".to_string()],
                &HashSet::new(),
                &HashSet::new(),
            )
            .await
            .expect("skill guidance")
            .expect("some");
        assert!(out.contains("mail-recipient-verify"));
        assert!(!out.contains("example.com"));
    }

    #[test]
    fn operational_skill_filter_excludes_meta_skill() {
        assert!(!super::is_operational_skill_id("skill-authoring-meta"));
        assert!(super::is_operational_skill_id("mail-recipient-verify"));
    }
}
