use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Task,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ToolCall {
    pub name: String,
    #[serde(default = "default_empty_object")]
    pub args: serde_json::Value,
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
}
