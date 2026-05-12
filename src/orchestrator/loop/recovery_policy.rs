use crate::executive::error::FcpError;

/// Classification output for tool errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFailureAction {
    TargetedSchemaRetry,
    Recoverable,
    Fatal,
}

/// Pure policy: classify a tool error without mutating orchestrator state.
///
/// Backend-specific recovery formatting (for example natural-language vs legacy recovery
/// banners) is applied in `tool_dispatch`; this function intentionally stays backend-agnostic.
///
/// `NetworkFault` is recoverable here (OAuth/API unreachable, etc.): the batch enters `Recover` so the
/// model can answer with `message_to_user` instead of aborting the orchestrator. LLM `generate`
/// failures are unrelated — they do not pass through this classifier.
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
            | FcpError::MoltbookResponseParse(_)
            | FcpError::NetworkFault(_)
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
    fn network_error_from_tool_is_recoverable() {
        let err = FcpError::NetworkFault("offline".to_string());
        let action = classify_tool_failure(&err, false);
        assert_eq!(action, ToolFailureAction::Recoverable);
    }

    #[test]
    fn moltbook_response_parse_never_targeted_schema_retry() {
        let err = FcpError::MoltbookResponseParse("bad remote body".into());
        assert_eq!(
            classify_tool_failure(&err, false),
            ToolFailureAction::Recoverable
        );
        assert_eq!(
            classify_tool_failure(&err, true),
            ToolFailureAction::Recoverable
        );
    }

    #[test]
    fn parse_fault_still_targeted_schema_retry_when_not_attempted() {
        let json_err: std::result::Result<serde_json::Value, _> = serde_json::from_str("{");
        let err = FcpError::ParseFault(json_err.unwrap_err());
        assert_eq!(
            classify_tool_failure(&err, false),
            ToolFailureAction::TargetedSchemaRetry
        );
    }
}
