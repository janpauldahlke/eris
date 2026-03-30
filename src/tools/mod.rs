use async_trait::async_trait;
use serde_json::Value;
use std::path::{Component, Path};
use crate::executive::error::{FcpError, Result};

/// Returns an Error if the path attempts to access the protected 00_Core directory.
pub fn validate_path_is_mutable(path_str: &str) -> Result<()> {
    let path = Path::new(path_str);
    
    for component in path.components() {
        if let Component::Normal(p) = component {
            if p == "00_Core" {
                return Err(FcpError::ToolFault {
                    tool_name: "gatekeeper".to_string(),
                    reason: "Access to 00_Core is strictly forbidden.".to_string(),
                });
            }
        }
    }
    
    Ok(())
}

#[async_trait]
pub trait Tool: Send + Sync {
    /// The canonical tool identifier (e.g., "vault:write")
    fn name(&self) -> &'static str;

    /// The JSON schema describing inputs for the LLM
    fn schema(&self) -> Value;

    /// Executes the action.
    async fn execute(&self, args: Value) -> Result<String>;
}

#[derive(Default)]
pub struct Gatekeeper {
    registry: std::collections::HashMap<String, Box<dyn Tool>>,
}

impl Gatekeeper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.registry.insert(tool.name().to_string(), tool);
    }

    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        match self.registry.get(name) {
            Some(tool) => tool.execute(args).await,
            None => Err(FcpError::ToolFault {
                tool_name: name.to_string(),
                reason: "Tool not found in registry".to_string(),
            }),
        }
    }

    pub fn get_tool_schemas_json(&self) -> String {
        let schemas: Vec<Value> = self.registry.values().map(|t| t.schema()).collect();
        serde_json::to_string_pretty(&schemas).unwrap_or_else(|_| "[]".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockPingTool;

    #[async_trait]
    impl Tool for MockPingTool {
        fn name(&self) -> &'static str {
            "ping"
        }

        fn schema(&self) -> Value {
            serde_json::json!({})
        }

        async fn execute(&self, _args: Value) -> Result<String> {
            Ok("pong".to_string())
        }
    }

    #[tokio::test]
    async fn test_gatekeeper_executes_registered_tool() {
        let mut gatekeeper = Gatekeeper::new();
        gatekeeper.register(Box::new(MockPingTool));

        // Test registered tool
        let res = gatekeeper.execute("ping", serde_json::json!({})).await;
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), "pong");

        // Test unregistered tool
        let res = gatekeeper.execute("fake:tool", serde_json::json!({})).await;
        assert!(res.is_err());
        match res {
            Err(FcpError::ToolFault { tool_name, reason }) => {
                assert_eq!(tool_name, "fake:tool");
                assert_eq!(reason, "Tool not found in registry");
            }
            _ => panic!("Expected ToolFault"),
        }
    }

    #[test]
    fn test_validate_path_allows_drops() {
        let res = validate_path_is_mutable("90_Drops/new_note.md");
        assert!(res.is_ok());
    }

    #[test]
    fn test_validate_path_rejects_core_root() {
        let res = validate_path_is_mutable("00_Core/Identity.md");
        assert!(res.is_err());
        match res {
            Err(FcpError::ToolFault { reason, .. }) => {
                assert_eq!(reason, "Access to 00_Core is strictly forbidden.");
            }
            _ => panic!("Expected ToolFault"),
        }
    }

    #[test]
    fn test_validate_path_rejects_core_traversal() {
        let res = validate_path_is_mutable("10_Projects/../00_Core/Identity.md");
        assert!(res.is_err());
        match res {
            Err(FcpError::ToolFault { reason, .. }) => {
                assert_eq!(reason, "Access to 00_Core is strictly forbidden.");
            }
            _ => panic!("Expected ToolFault"),
        }
    }
}
