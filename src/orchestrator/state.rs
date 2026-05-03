use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Chat,
    Reflect,
    Idle,
    Recover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum LoopAction {
    Reflect,
    Idle,
    /// LLMs sometimes emit `"Process"` from training drift; map it to Task.
    #[serde(alias = "Process")]
    Task,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ToolCall {
    /// LLMs sometimes emit `action` instead of `name`.
    #[serde(alias = "action")]
    pub name: String,
    /// OpenAI-style tool calls often use `arguments` (string or object); we store the object in `args`.
    #[serde(default = "default_empty_object", alias = "arguments")]
    pub args: serde_json::Value,
    /// Top-level task id (e.g. agenda) folded into `args` during normalization.
    #[serde(default)]
    pub id: Option<String>,
}

fn default_empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LlmResponse {
    #[serde(default)]
    pub thought: String,
    status: Option<LoopAction>,
    pub message_to_user: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

impl LlmResponse {
    pub fn has_explicit_status(&self) -> bool {
        self.status.is_some()
    }

    /// Maps common LLM drift (`action`, top-level `id`) into the tool schema expected by dispatch.
    pub fn normalize_tool_calls(&mut self) {
        for tc in &mut self.tool_calls {
            if tc.name == "agenda:complete" {
                if let Some(obj) = tc.args.as_object_mut() {
                    if let Some(ref top) = tc.id {
                        if !obj.contains_key("task_id") {
                            obj.insert(
                                "task_id".to_string(),
                                serde_json::Value::String(top.clone()),
                            );
                        }
                    }
                    if !obj.contains_key("result_summary") {
                        obj.insert(
                            "result_summary".to_string(),
                            serde_json::json!("User confirmed completion"),
                        );
                    }
                }
            }
            tc.id = None;
        }
    }

    /// If the LLM omitted `status`, infer it from the other fields:
    ///   - tool_calls non-empty → Reflect
    ///   - message_to_user present → Idle
    ///   - otherwise → Task
    pub fn status(&self) -> LoopAction {
        self.status.unwrap_or_else(|| {
            if !self.tool_calls.is_empty() {
                LoopAction::Reflect
            } else if self.message_to_user.as_ref().is_some_and(|m| !m.trim().is_empty()) {
                LoopAction::Idle
            } else {
                LoopAction::Task
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoopDirective {
    ExecuteTools(Vec<ToolCall>),
    HaltAndAwaitInput(Option<String>),
    ShiftToReflection,
    RecoverFromFuckup(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolIntentStatus {
    Pending,
    Success,
    FailedRecoverable,
    FailedFatal,
}

#[derive(Debug, Clone)]
pub struct ToolIntentTicket {
    pub intent_id: String,
    pub tool_name: String,
    pub args: Value,
    pub status: ToolIntentStatus,
    pub attempt_count: u8,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_llm_response_deserialization() {
        let raw_json = r#"{
            "thought": "I need to write a file",
            "status": "Task",
            "tool_calls": [{
                "name": "vault:write",
                "args": {
                    "path": "test.md"
                }
            }]
        }"#;

        let response: LlmResponse = serde_json::from_str(raw_json).unwrap();

        assert_eq!(response.thought, "I need to write a file");
        assert_eq!(response.status(), LoopAction::Task);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "vault:write");
        
        let expected_args = serde_json::json!({
            "path": "test.md"
        });
        assert_eq!(response.tool_calls[0].args, expected_args);
        assert_eq!(response.message_to_user, None);
    }

    #[test]
    fn test_llm_response_missing_status_with_message() {
        let json = r#"{
            "thought": "Done thinking",
            "message_to_user": "Here is your answer.",
            "tool_calls": []
        }"#;
        let response: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.status(), LoopAction::Idle);
    }

    #[test]
    fn test_llm_response_missing_status_with_tools() {
        let json = r#"{
            "thought": "Need vault",
            "tool_calls": [{"name": "vault:read", "args": {"path": "x.md"}}]
        }"#;
        let response: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.status(), LoopAction::Reflect);
    }

    #[test]
    fn test_llm_response_missing_status_bare() {
        let json = r#"{"thought": "planning"}"#;
        let response: LlmResponse = serde_json::from_str(json).unwrap();
        assert!(!response.has_explicit_status());
        assert_eq!(response.status(), LoopAction::Task);
    }

    #[test]
    fn test_llm_response_process_alias_deserializes_as_task() {
        let json = r#"{
            "thought": "mid-step",
            "status": "Process",
            "tool_calls": []
        }"#;
        let response: LlmResponse = serde_json::from_str(json).unwrap();
        assert!(response.has_explicit_status());
        assert_eq!(response.status(), LoopAction::Task);
    }

    #[test]
    fn test_tool_call_arguments_alias_deserializes_like_args() {
        let json = r#"{
            "thought": "news",
            "status": "Task",
            "tool_calls": [{"name": "news:today", "arguments": {"category": "politics"}}]
        }"#;
        let response: LlmResponse = serde_json::from_str(json).expect("parse");
        assert_eq!(response.tool_calls[0].args, json!({"category": "politics"}));
    }

    #[test]
    fn test_tool_call_action_alias_and_agenda_complete_normalization() {
        let json = r#"{
            "thought": "mark done",
            "status": "Task",
            "tool_calls": [{"id": "4049", "action": "agenda:complete"}]
        }"#;
        let mut response: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.tool_calls[0].name, "agenda:complete");
        response.normalize_tool_calls();
        let args = response.tool_calls[0].args.as_object().unwrap();
        assert_eq!(args.get("task_id").and_then(|v| v.as_str()), Some("4049"));
        assert!(args.get("result_summary").is_some());
        assert_eq!(response.tool_calls[0].id, None);
    }
}
