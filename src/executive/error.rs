use thiserror::Error;

#[derive(Error, Debug)]
pub enum FcpError {
    #[error("I/O Fault: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Configuration Fault: {0}")]
    Config(String),

    #[error("Engine Fault: {0}")]
    Engine(String),

    #[error("Orchestration Fault: {0}")]
    Orchestration(String),

    #[error("Tool Fault [{tool_name}]: {reason}")]
    Tool { tool_name: String, reason: String },
}

pub type Result<T> = std::result::Result<T, FcpError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_formatting() {
        let err = FcpError::Config("missing env".into());
        assert_eq!(err.to_string(), "Configuration Fault: missing env");

        let err2 = FcpError::Tool {
            tool_name: "test_tool".into(),
            reason: "permission denied".into(),
        };
        assert_eq!(err2.to_string(), "Tool Fault [test_tool]: permission denied");
    }
}
