use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Chat,
    Reflect,
    Idle,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoopAction {
    ContinueTask,
    WaitForUser,
    InitiateReflection,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LlmResponse {
    pub status: LoopAction,
    pub thoughts: Option<String>,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub response: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_response_deserialization() {
        let raw_json = r#"{
            "status": "CONTINUE_TASK",
            "tool_name": "vault:write",
            "tool_args": {
                "path": "test.md"
            }
        }"#;

        let response: LlmResponse = serde_json::from_str(raw_json).unwrap();

        assert_eq!(response.status, LoopAction::ContinueTask);
        assert_eq!(response.tool_name.as_deref(), Some("vault:write"));
        
        let expected_args = serde_json::json!({
            "path": "test.md"
        });
        assert_eq!(response.tool_args, Some(expected_args));
        assert_eq!(response.thoughts, None);
        assert_eq!(response.response, None);
    }
}
