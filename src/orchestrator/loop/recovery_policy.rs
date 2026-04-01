use crate::executive::error::FcpError;

/// Classification output for tool errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFailureAction {
    TargetedSchemaRetry,
    Recoverable,
    Fatal,
}

/// Pure policy: classify a tool error without mutating orchestrator state.
pub fn classify_tool_failure(err: &FcpError, schema_already_attempted: bool) -> ToolFailureAction {
    let schema_or_parse = matches!(err, FcpError::SchemaViolation(_) | FcpError::ParseFault(_));
    if schema_or_parse && !schema_already_attempted {
        return ToolFailureAction::TargetedSchemaRetry;
    }

    if matches!(
        err,
        FcpError::ToolFault { .. }
            | FcpError::SchemaViolation(_)
            | FcpError::Io(_)
            | FcpError::ParseFault(_)
    ) {
        ToolFailureAction::Recoverable
    } else {
        ToolFailureAction::Fatal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_error_without_prior_attempt_yields_targeted_retry() {
        let err = FcpError::SchemaViolation("bad args".to_string());
        let action = classify_tool_failure(&err, false);
        assert_eq!(action, ToolFailureAction::TargetedSchemaRetry);
    }

    #[test]
    fn schema_error_after_retry_is_recoverable() {
        let err = FcpError::SchemaViolation("bad args".to_string());
        let action = classify_tool_failure(&err, true);
        assert_eq!(action, ToolFailureAction::Recoverable);
    }

    #[test]
    fn network_error_is_fatal() {
        let err = FcpError::NetworkFault("offline".to_string());
        let action = classify_tool_failure(&err, false);
        assert_eq!(action, ToolFailureAction::Fatal);
    }
}
