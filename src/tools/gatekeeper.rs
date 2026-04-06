use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use jsonschema::JSONSchema;
use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;
use crate::tools::context_view_hint::ToolContextViewHint;
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
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name();
        if !Self::is_tool_allowed_in_any_state(name) {
            tracing::warn!(tool = name, "Registered tool is not allowed in any state");
        }
        self.registry.insert(name.to_string(), tool);
    }

    fn state_allows_tool(state: &AgentState, tool_name: &str) -> bool {
        match state {
            AgentState::Chat => !matches!(tool_name, "agenda:complete"),
            AgentState::Reflect => matches!(tool_name, "memory:stage" | "memory:staged_list" | "memory:commit" | "memory:commit_all" | "memory:query" | "vault:read" | "vault:list" | "agenda:push" | "agenda:list" | "agenda:remove" | "agenda:remind_at" | "web:artifact_query" | "system:health" | "clock:now" | "clock:timer" | "clock:alarm" | "weather:current" | "weather:forecast" | "wiki:summary" | "mail:check" | "mail:read"),
            AgentState::Idle => matches!(tool_name, "memory:staged_list" | "memory:commit" | "memory:commit_all" | "memory:query" | "vault:read" | "vault:write" | "vault:list" | "agenda:list" | "agenda:complete" | "agenda:remove" | "agenda:remind_at" | "web:fetch" | "web:artifact_query" | "system:health" | "clock:now" | "clock:timer" | "clock:alarm" | "weather:current" | "weather:forecast" | "wiki:summary" | "mail:check" | "mail:read" | "mail:write"),
            AgentState::Recover => true,
        }
    }

    pub fn is_tool_allowed_in_any_state(tool_name: &str) -> bool {
        Self::state_allows_tool(&AgentState::Chat, tool_name)
            || Self::state_allows_tool(&AgentState::Reflect, tool_name)
            || Self::state_allows_tool(&AgentState::Idle, tool_name)
            || Self::state_allows_tool(&AgentState::Recover, tool_name)
    }

    pub fn all_tool_descriptions(&self) -> Vec<(String, String)> {
        self.registry
            .values()
            .map(|t| (t.name().to_string(), t.description().to_string()))
            .collect()
    }

    pub fn registered_tool_names(&self) -> Vec<String> {
        let mut names = self.registry.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    /// Trait defaults for each registered tool, merged with `overrides` (config wins).
    pub fn merge_context_view_hints(
        &self,
        overrides: &HashMap<String, ToolContextViewHint>,
    ) -> HashMap<String, ToolContextViewHint> {
        let mut m = HashMap::with_capacity(self.registry.len() + overrides.len());
        for (name, tool) in &self.registry {
            m.insert(name.clone(), tool.context_view_hint());
        }
        for (k, v) in overrides {
            m.insert(k.clone(), *v);
        }
        m
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
        tracing::info!(tool = name, state = ?state, "Gatekeeper: checking tool authorization");

        if !Self::state_allows_tool(state, name) {
            tracing::warn!(tool = name, state = ?state, "Gatekeeper: tool not authorized in current state");
            return Err(FcpError::SchemaViolation(format!("Tool '{}' not authorized in state {:?}", name, state)));
        }
        let tool = self.registry.get(name).ok_or_else(|| {
            tracing::warn!(tool = name, registered_tools = ?self.registry.keys().collect::<Vec<_>>(), "Gatekeeper: tool not found in registry");
            FcpError::ToolFault { tool_name: name.to_string(), reason: "Tool not found".to_string() }
        })?;

        let schema_value = serde_json::to_value(tool.parameters_schema()).map_err(|e| FcpError::Config(e.to_string()))?;
        let compiled_schema = JSONSchema::options().compile(&schema_value)
            .map_err(|e| FcpError::Config(format!("Failed to compile JSON schema: {}", e)))?;

        if let Err(errors) = compiled_schema.validate(&args) {
            let msg = errors.map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
            tracing::warn!(tool = name, validation_error = %msg, args = %args, "Gatekeeper: schema validation failed");
            return Err(FcpError::SchemaViolation(format!("JSON Schema Validation Failed: {}", msg)));
        }

        tracing::debug!(tool = name, args = %args, "Gatekeeper: executing tool");
        let result = tool.execute(args).await?;
        tracing::debug!(tool = name, result_len = result.len(), "Gatekeeper: tool execution complete");

        if state == &AgentState::Recover && result.trim().is_empty() {
            tracing::warn!(tool = name, "Gatekeeper: semantic guard triggered (empty result in Recover)");
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

    #[tokio::test]
    async fn test_gatekeeper_blocks_agenda_complete_in_chat() {
        let mut gatekeeper = Gatekeeper::new();
        
        struct MockAgendaComplete;
        #[async_trait]
        impl Tool for MockAgendaComplete {
            fn name(&self) -> &'static str { "agenda:complete" }
            fn description(&self) -> &'static str { "complete" }
            fn parameters_schema(&self) -> RootSchema { schemars::schema_for!(EmptyArgs) }
            async fn execute(&self, _args: Value) -> Result<String> { Ok("done".to_string()) }
        }
        
        gatekeeper.register(Arc::new(MockAgendaComplete));

        let res = gatekeeper.execute_tool(&AgentState::Chat, "agenda:complete", json!({})).await;
        assert!(res.is_err());
        match res {
            Err(FcpError::SchemaViolation(msg)) => {
                assert!(msg.contains("not authorized"));
            }
            _ => panic!("Expected SchemaViolation"),
        }
    }

    #[test]
    fn test_policy_covers_all_current_tools() {
        let known_tools = [
            "vault:read",
            "vault:write",
            "vault:list",
            "agenda:push",
            "agenda:list",
            "agenda:complete",
            "agenda:remove",
            "agenda:remind_at",
            "web:fetch",
            "web:artifact_query",
            "memory:stage",
            "memory:staged_list",
            "memory:commit",
            "memory:commit_all",
            "memory:query",
            "system:health",
            "clock:now",
            "clock:timer",
            "clock:alarm",
            "weather:current",
            "weather:forecast",
            "wiki:summary",
            "mail:check",
            "mail:read",
            "mail:write",
        ];

        for tool in known_tools {
            assert!(
                Gatekeeper::is_tool_allowed_in_any_state(tool),
                "tool should be allowed in at least one state: {}",
                tool
            );
        }
    }
}
