use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use jsonschema::JSONSchema;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;
use crate::tools::traits::Tool;

pub struct Gatekeeper {
    registry: HashMap<String, Arc<dyn Tool>>,
}

impl Default for Gatekeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Gatekeeper {
    pub fn new() -> Self { Self { registry: HashMap::new() } }
    pub fn register(&mut self, tool: Arc<dyn Tool>) { self.registry.insert(tool.name().to_string(), tool); }

    fn state_allows_tool(state: &AgentState, tool_name: &str) -> bool {
        match state {
            AgentState::Chat => true,
            AgentState::Reflect => matches!(tool_name, "memory:stage" | "memory:commit" | "vault:read"),
            AgentState::Idle => matches!(tool_name, "memory:commit" | "vault:read"),
            AgentState::Recover => true,
        }
    }

    pub fn get_allowed_tools(&self, state: &AgentState) -> Vec<Value> {
        self.registry.values()
            .filter(|tool| Self::state_allows_tool(state, tool.name()))
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.parameters_schema()
                    }
                })
            }).collect()
    }

    pub async fn execute_tool(&self, state: &AgentState, name: &str, args: Value) -> Result<String> {
        if !Self::state_allows_tool(state, name) {
            return Err(FcpError::SchemaViolation(format!("Tool '{}' not authorized in state {:?}", name, state)));
        }
        let tool = self.registry.get(name).ok_or_else(|| {
            FcpError::ToolFault { tool_name: name.to_string(), reason: "Tool not found".to_string() }
        })?;

        let schema_value = serde_json::to_value(tool.parameters_schema()).map_err(|e| FcpError::Config(e.to_string()))?;
        let compiled_schema = JSONSchema::options().compile(&schema_value)
            .map_err(|e| FcpError::Config(format!("Failed to compile JSON schema: {}", e)))?;

        if let Err(errors) = compiled_schema.validate(&args) {
            let msg = errors.map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
            return Err(FcpError::SchemaViolation(format!("JSON Schema Validation Failed: {}", msg)));
        }

        let result = tool.execute(args).await?;

        if state == &AgentState::Recover && result.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: name.to_string(),
                reason: "Semantic Guard: Tool returned zero logic results during recovery".to_string()
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use schemars::schema::RootSchema;
    use serde::Deserialize;
    use async_trait::async_trait;
    use crate::tools::validation::validate_path_is_mutable;

    #[derive(JsonSchema, Deserialize)]
    struct PingArgs {
        message: String,
    }

    struct MockPingTool;

    #[async_trait]
    impl Tool for MockPingTool {
        fn name(&self) -> &'static str {
            "ping"
        }

        fn description(&self) -> &'static str {
            "A simple ping tool"
        }

        fn parameters_schema(&self) -> RootSchema {
            schemars::schema_for!(PingArgs)
        }

        async fn execute(&self, args: Value) -> Result<String> {
            let parsed: PingArgs = serde_json::from_value(args).unwrap();
            Ok(format!("pong: {}", parsed.message))
        }
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct VaultWriteArgs {
        path: String,
        content: String,
    }

    struct MockVaultWrite;

    #[async_trait]
    impl Tool for MockVaultWrite {
        fn name(&self) -> &'static str {
            "vault:write"
        }

        fn description(&self) -> &'static str {
            "Write to vault"
        }

        fn parameters_schema(&self) -> RootSchema {
            schemars::schema_for!(VaultWriteArgs)
        }

        async fn execute(&self, args: Value) -> Result<String> {
            let parsed: VaultWriteArgs = serde_json::from_value(args).unwrap();
            validate_path_is_mutable(&parsed.path)?;
            Ok(format!("written to {}", parsed.path))
        }
    }

    #[tokio::test]
    async fn test_gatekeeper_executes_registered_tool() {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(MockPingTool));

        let res = gatekeeper.execute_tool(&AgentState::Chat, "ping", json!({"message": "hello"})).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "pong: hello");

        let res = gatekeeper.execute_tool(&AgentState::Chat, "fake:tool", json!({})).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_gatekeeper_schema_violation_missing_args() {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(MockPingTool));

        let res = gatekeeper.execute_tool(&AgentState::Chat, "ping", json!({})).await;
        assert!(res.is_err());
        match res {
            Err(FcpError::SchemaViolation(msg)) => {
                assert!(msg.contains("JSON Schema Validation Failed"));
            }
            _ => panic!("Expected SchemaViolation"),
        }
    }

    #[tokio::test]
    async fn test_gatekeeper_unauthorized_tool_in_reflect() {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(MockVaultWrite));

        let res = gatekeeper.execute_tool(&AgentState::Reflect, "vault:write", json!({"path": "test.md", "content": "test"})).await;
        assert!(res.is_err());
        match res {
            Err(FcpError::SchemaViolation(msg)) => {
                assert!(msg.contains("not authorized"));
            }
            _ => panic!("Expected SchemaViolation"),
        }
    }

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    #[tokio::test]
    async fn test_gatekeeper_semantic_guard_empty_result() {
        struct EmptyTool;
        #[async_trait]
        impl Tool for EmptyTool {
            fn name(&self) -> &'static str { "empty" }
            fn description(&self) -> &'static str { "empty" }
            fn parameters_schema(&self) -> RootSchema { schemars::schema_for!(EmptyArgs) }
            async fn execute(&self, _args: Value) -> Result<String> { Ok("   ".to_string()) }
        }

        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(EmptyTool));

        let res = gatekeeper.execute_tool(&AgentState::Recover, "empty", json!({})).await;
        assert!(res.is_err());
        match res {
            Err(FcpError::ToolFault { reason, .. }) => {
                assert!(reason.contains("Semantic Guard"));
            }
            _ => panic!("Expected ToolFault"),
        }
    }
}
