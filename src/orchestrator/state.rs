use serde::Deserialize;

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
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LlmResponse {
    pub thought: String,
    pub status: LoopAction,
    pub message_to_user: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

/// Lightweight response shape for the conversational pre-check (no tools).
#[derive(Debug, Clone, Deserialize)]
pub struct ConversationalResponse {
    pub thought: String,
    pub message_to_user: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoopDirective {
    ExecuteTools(Vec<ToolCall>),
    HaltAndAwaitInput(Option<String>),
    ShiftToReflection,
    RecoverFromFuckup(String),
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
        assert_eq!(response.status, LoopAction::Task);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "vault:write");
        
        let expected_args = serde_json::json!({
            "path": "test.md"
        });
        assert_eq!(response.tool_calls[0].args, expected_args);
        assert_eq!(response.message_to_user, None);
    }
}
