use crate::executive::error::{FcpError, Result};
use crate::orchestrator::state::AgentState;
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::traits::Tool;
use jsonschema::JSONSchema;
use schemars::schema::RootSchema;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Gatekeeper {
    registry: HashMap<String, Arc<dyn Tool>>,
}

impl Default for Gatekeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Gatekeeper {
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
        }
    }
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
            AgentState::Reflect => matches!(
                tool_name,
                "memory:stage"
                    | "memory:staged_list"
                    | "memory:commit"
                    | "memory:commit_all"
                    | "memory:query"
                    | "vault:read"
                    | "vault:list"
                    | "vault:search"
                    | "vault:taglist"
                    | "skills:list"
                    | "skills:read"
                    | "skills:create"
                    | "agenda:push"
                    | "agenda:list"
                    | "agenda:remove"
                    | "agenda:remind_at"
                    | "agenda:remind_self"
                    | "web:find"
                    | "system:health"
                    | "clock:now"
                    | "clock:timer"
                    | "clock:alarm"
                    | "weather:current"
                    | "weather:forecast"
                    | "wiki:summary"
                    | "db:find_connections"
                    | "mail:check"
                    | "mail:read"
                    | "mail:digest"
                    | "calendar:list"
                    | "calendar:get"
                    | "moltbook:status"
                    | "moltbook:home"
                    | "moltbook:feed"
                    | "moltbook:search"
                    | "moltbook:comments"
                    | "moltbook:verify"
                    | "moltbook:dm"
            ),
            AgentState::Idle => matches!(
                tool_name,
                "memory:stage"
                    | "memory:staged_list"
                    | "memory:commit"
                    | "memory:commit_all"
                    | "memory:query"
                    | "vault:read"
                    | "vault:write"
                    | "vault:list"
                    | "vault:search"
                    | "vault:taglist"
                    | "skills:list"
                    | "skills:read"
                    | "skills:create"
                    | "agenda:list"
                    | "agenda:complete"
                    | "agenda:remove"
                    | "agenda:remind_at"
                    | "agenda:remind_self"
                    | "web:fetch"
                    | "web:search"
                    | "news:today"
                    | "web:find"
                    | "system:health"
                    | "clock:now"
                    | "clock:timer"
                    | "clock:alarm"
                    | "weather:current"
                    | "weather:forecast"
                    | "wiki:summary"
                    | "db:find_connections"
                    | "mail:check"
                    | "mail:read"
                    | "mail:digest"
                    | "mail:write"
                    | "mail:delete"
                    | "mail:move"
                    | "calendar:list"
                    | "calendar:get"
                    | "calendar:create"
                    | "calendar:update"
                    | "calendar:delete"
                    | "moltbook:register"
                    | "moltbook:status"
                    | "moltbook:home"
                    | "moltbook:feed"
                    | "moltbook:search"
                    | "moltbook:comments"
                    | "moltbook:comment"
                    | "moltbook:vote"
                    | "moltbook:post"
                    | "moltbook:verify"
                    | "moltbook:notifications_read"
                    | "moltbook:dm"
            ),
            AgentState::Recover => matches!(
                tool_name,
                "memory:stage"
                    | "memory:staged_list"
                    | "memory:commit"
                    | "memory:commit_all"
                    | "memory:query"
                    | "vault:read"
                    | "vault:list"
                    | "vault:search"
                    | "vault:taglist"
                    | "skills:list"
                    | "skills:read"
                    | "news:today"
                    | "web:find"
                    | "system:health"
                    | "clock:now"
            ),
        }
    }

    pub fn is_tool_allowed_in_any_state(tool_name: &str) -> bool {
        Self::state_allows_tool(&AgentState::Chat, tool_name)
            || Self::state_allows_tool(&AgentState::Reflect, tool_name)
            || Self::state_allows_tool(&AgentState::Idle, tool_name)
            || Self::state_allows_tool(&AgentState::Recover, tool_name)
    }

    /// Authorization state for tool dispatch while the orchestrator may be in `Recover`.
    ///
    /// Protocol-parse recovery and schema-retry recovery both run tool rounds in `Recover`.
    /// Chat-only tools (e.g. `web:fetch`) must still execute so a recovery hop can finish the
    /// user's request instead of failing with "not authorized in state Recover".
    pub fn dispatch_authorization_state(
        orchestrator_state: &AgentState,
        tool_name: &str,
        force_full_tool_schemas_in_llm_view: bool,
    ) -> AgentState {
        if force_full_tool_schemas_in_llm_view {
            return AgentState::Chat;
        }
        if *orchestrator_state == AgentState::Recover
            && Self::state_allows_tool(&AgentState::Chat, tool_name)
            && !Self::state_allows_tool(&AgentState::Recover, tool_name)
        {
            return AgentState::Chat;
        }
        *orchestrator_state
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

    /// Returns `true` if the named tool opts out of per-turn duplicate suppression.
    pub fn tool_allows_repeat(&self, name: &str) -> bool {
        self.registry
            .get(name)
            .is_some_and(|t| t.allow_repeat_in_turn())
    }

    /// Parameter JSON Schema root for a registered tool (for recovery / diagnostics).
    pub fn parameters_root_schema_for(&self, name: &str) -> Option<RootSchema> {
        self.registry.get(name).map(|t| t.parameters_schema())
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
        self.registry
            .values()
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
            })
            .collect()
    }

    /// Registered tool names allowed in `state` whose name starts with `prefix` (e.g. `"moltbook:"`).
    pub fn allowed_tool_names_with_prefix(&self, state: &AgentState, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .registry
            .keys()
            .filter(|name| name.starts_with(prefix) && Self::state_allows_tool(state, name))
            .cloned()
            .collect();
        names.sort();
        names
    }

    pub async fn execute_tool(
        &self,
        state: &AgentState,
        name: &str,
        args: Value,
    ) -> Result<String> {
        tracing::info!(tool = name, state = ?state, "Gatekeeper: checking tool authorization");

        if !Self::state_allows_tool(state, name) {
            tracing::warn!(tool = name, state = ?state, "Gatekeeper: tool not authorized in current state");
            return Err(FcpError::ToolFault {
                tool_name: name.to_string(),
                reason: format!("Tool not authorized in state {:?}", state),
            });
        }
        let tool = self.registry.get(name).ok_or_else(|| {
            tracing::warn!(tool = name, registered_tools = ?self.registry.keys().collect::<Vec<_>>(), "Gatekeeper: tool not found in registry");
            FcpError::ToolFault { tool_name: name.to_string(), reason: "Tool not found".to_string() }
        })?;

        let args = normalize_tool_args(name, args);

        let schema_value = serde_json::to_value(tool.parameters_schema())
            .map_err(|e| FcpError::Config(e.to_string()))?;
        let compiled_schema = JSONSchema::options()
            .compile(&schema_value)
            .map_err(|e| FcpError::Config(format!("Failed to compile JSON schema: {}", e)))?;

        if let Err(errors) = compiled_schema.validate(&args) {
            let msg = errors.map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
            tracing::warn!(tool = name, validation_error = %msg, args = %args, "Gatekeeper: schema validation failed");
            return Err(FcpError::SchemaViolation(format!(
                "JSON Schema Validation Failed: {}",
                msg
            )));
        }

        tracing::debug!(tool = name, args = %args, "Gatekeeper: executing tool");
        let result = tool.execute(args).await?;
        tracing::debug!(
            tool = name,
            result_len = result.len(),
            "Gatekeeper: tool execution complete"
        );

        if state == &AgentState::Recover && result.trim().is_empty() {
            tracing::warn!(
                tool = name,
                "Gatekeeper: semantic guard triggered (empty result in Recover)"
            );
            return Err(FcpError::ToolFault {
                tool_name: name.to_string(),
                reason: "Semantic Guard: Tool returned zero logic results during recovery"
                    .to_string(),
            });
        }
        Ok(result)
    }
}

/// Coerce common LLM mistakes before JSON Schema validation (e.g. `""` for omitted optionals).
fn normalize_tool_args(tool_name: &str, mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };
    if tool_name == "news:today" {
        for key in ["category", "homepage_url"] {
            if obj.get(key).and_then(|v| v.as_str()).is_some_and(|s| s.trim().is_empty()) {
                obj.remove(key);
            }
        }
    }
    if tool_name == "web:search" {
        for alias in ["q", "search_query", "search"] {
            if let Some(val) = obj.remove(alias) {
                if !obj.contains_key("query") {
                    obj.insert("query".to_string(), val);
                }
            }
        }
        if let Some(q) = obj.get("query").and_then(|v| v.as_str()) {
            if q.trim().is_empty() {
                obj.remove("query");
            }
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::validation::validate_path_is_mutable;
    use async_trait::async_trait;
    use schemars::JsonSchema;
    use schemars::schema::RootSchema;
    use serde::Deserialize;

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

        let res = gatekeeper
            .execute_tool(&AgentState::Chat, "ping", json!({"message": "hello"}))
            .await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "pong: hello");

        let res = gatekeeper
            .execute_tool(&AgentState::Chat, "fake:tool", json!({}))
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_gatekeeper_schema_violation_missing_args() {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(MockPingTool));

        let res = gatekeeper
            .execute_tool(&AgentState::Chat, "ping", json!({}))
            .await;
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

        let res = gatekeeper
            .execute_tool(
                &AgentState::Reflect,
                "vault:write",
                json!({"path": "test.md", "content": "test"}),
            )
            .await;
        assert!(res.is_err());
        match res {
            Err(FcpError::ToolFault { reason, .. }) => {
                assert!(reason.contains("not authorized"));
            }
            _ => panic!("Expected ToolFault"),
        }
    }

    #[test]
    fn dispatch_authorization_state_elevates_chat_only_web_tools_in_recover() {
        assert_eq!(
            Gatekeeper::dispatch_authorization_state(
                &AgentState::Recover,
                "web:fetch",
                false,
            ),
            AgentState::Chat
        );
        assert_eq!(
            Gatekeeper::dispatch_authorization_state(
                &AgentState::Recover,
                "web:find",
                false,
            ),
            AgentState::Recover
        );
        assert_eq!(
            Gatekeeper::dispatch_authorization_state(
                &AgentState::Recover,
                "web:fetch",
                true,
            ),
            AgentState::Chat
        );
    }

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    #[tokio::test]
    async fn test_gatekeeper_semantic_guard_empty_result() {
        struct EmptyTool;
        #[async_trait]
        impl Tool for EmptyTool {
            fn name(&self) -> &'static str {
                "web:find"
            }
            fn description(&self) -> &'static str {
                "empty"
            }
            fn parameters_schema(&self) -> RootSchema {
                schemars::schema_for!(EmptyArgs)
            }
            async fn execute(&self, _args: Value) -> Result<String> {
                Ok("   ".to_string())
            }
        }

        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Arc::new(EmptyTool));

        let res = gatekeeper
            .execute_tool(&AgentState::Recover, "web:find", json!({}))
            .await;
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
            fn name(&self) -> &'static str {
                "agenda:complete"
            }
            fn description(&self) -> &'static str {
                "complete"
            }
            fn parameters_schema(&self) -> RootSchema {
                schemars::schema_for!(EmptyArgs)
            }
            async fn execute(&self, _args: Value) -> Result<String> {
                Ok("done".to_string())
            }
        }

        gatekeeper.register(Arc::new(MockAgendaComplete));

        let res = gatekeeper
            .execute_tool(&AgentState::Chat, "agenda:complete", json!({}))
            .await;
        assert!(res.is_err());
        match res {
            Err(FcpError::ToolFault { reason, .. }) => {
                assert!(reason.contains("not authorized"));
            }
            _ => panic!("Expected ToolFault"),
        }
    }

    #[test]
    fn normalize_web_search_aliases_q_to_query() {
        let args = normalize_tool_args(
            "web:search",
            json!({"q": "bundesliga letzter spieltag"}),
        );
        assert_eq!(
            args.get("query").and_then(|v| v.as_str()),
            Some("bundesliga letzter spieltag")
        );
        assert!(args.get("q").is_none());
    }

    #[test]
    fn normalize_news_today_strips_empty_category() {
        let args = normalize_tool_args(
            "news:today",
            json!({"homepage_url": "https://www.bbc.com/news", "category": ""}),
        );
        assert!(args.get("category").is_none());
        assert_eq!(
            args.get("homepage_url").and_then(|v| v.as_str()),
            Some("https://www.bbc.com/news")
        );
    }

    #[test]
    fn test_policy_covers_all_current_tools() {
        let known_tools = [
            "vault:read",
            "vault:write",
            "vault:list",
            "vault:search",
            "vault:taglist",
            "skills:list",
            "skills:read",
            "skills:create",
            "agenda:push",
            "agenda:list",
            "agenda:complete",
            "agenda:remove",
            "agenda:remind_at",
            "agenda:remind_self",
            "web:fetch",
            "web:search",
            "news:today",
            "web:find",
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
            "db:find_connections",
            "mail:check",
            "mail:read",
            "mail:digest",
            "mail:write",
            "mail:delete",
            "mail:move",
            "calendar:list",
            "calendar:get",
            "calendar:create",
            "calendar:update",
            "calendar:delete",
            "moltbook:register",
            "moltbook:status",
            "moltbook:home",
            "moltbook:feed",
            "moltbook:search",
            "moltbook:comments",
            "moltbook:comment",
            "moltbook:vote",
            "moltbook:post",
            "moltbook:verify",
            "moltbook:notifications_read",
            "moltbook:dm",
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
