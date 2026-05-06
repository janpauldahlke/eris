use thiserror::Error;

#[derive(Error, Debug)]
pub enum FcpError {
    #[error("I/O Fault: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration Fault: {0}")]
    Config(String),

    #[error("Workspace Fault [{workspace}]: {reason}")]
    WorkspaceFault { workspace: String, reason: String },

    #[error("LLM Engine Fault: {0}")]
    EngineFault(String),

    #[error("Network/Daemon Unreachable: {0}")]
    NetworkFault(String),

    #[error("Token Limit Breached: prompt requires {requested}, context is {available}")]
    ContextExhaustion { requested: usize, available: usize },

    #[error("Gatekeeper Validation Failed: {0}")]
    SchemaViolation(String),

    #[error("User Cancellation: {0}")]
    Cancellation(String),

    #[error("Tool Execution Failed [{tool_name}]: {reason}")]
    ToolFault { tool_name: String, reason: String },

    #[error("JSON Parse Fault: {0}")]
    ParseFault(#[from] serde_json::Error),

    #[error("Qdrant Daemon Offline: {0}")]
    VectorDbOffline(String),

    #[error("Embedding Generation Failed: {0}")]
    EmbeddingFault(String),

    #[error("Execution Interrupted")]
    Interrupted,
}

pub type Result<T> = std::result::Result<T, FcpError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::from_str;

    #[test]
    fn test_error_formatting() {
        let err = FcpError::Config("missing env".into());
        assert_eq!(err.to_string(), "Configuration Fault: missing env");

        let err2 = FcpError::ToolFault {
            tool_name: "test_tool".into(),
            reason: "permission denied".into(),
        };
        assert_eq!(
            err2.to_string(),
            "Tool Execution Failed [test_tool]: permission denied"
        );

        let err3 = FcpError::WorkspaceFault {
            workspace: "isolated_env".into(),
            reason: "partition not found".into(),
        };
        assert_eq!(
            err3.to_string(),
            "Workspace Fault [isolated_env]: partition not found"
        );

        // Force a ParseFault
        let json_err: std::result::Result<serde_json::Value, _> = from_str("{invalid}");
        let err4 = FcpError::ParseFault(json_err.unwrap_err());
        assert!(err4.to_string().starts_with("JSON Parse Fault:"));

        let err5 = FcpError::Interrupted;
        assert_eq!(err5.to_string(), "Execution Interrupted");
    }

    #[test]
    fn test_network_fault_mapping() {
        let err = FcpError::NetworkFault("timeout from daemon".into());
        assert_eq!(
            err.to_string(),
            "Network/Daemon Unreachable: timeout from daemon"
        );
    }
}
