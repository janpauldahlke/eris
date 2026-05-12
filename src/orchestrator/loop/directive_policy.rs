use crate::orchestrator::context::resolved_tool_recovery::PROTOCOL_FAULT_PREFIX;
use crate::orchestrator::r#loop::transition::StateTransition;
use crate::orchestrator::state::LoopDirective;

/// Pure policy: map parsed LLM directives to coordinator transitions.
pub fn decide_transition_from_directive(directive: LoopDirective) -> StateTransition {
    match directive {
        LoopDirective::HaltAndAwaitInput(_) => StateTransition::Halt,
        LoopDirective::RecoverFromFuckup(msg) => StateTransition::Recover {
            message: format!("{PROTOCOL_FAULT_PREFIX}: {msg}"),
            schema_retry: false,
        },
        LoopDirective::ShiftToReflection => StateTransition::ShiftToReflection,
        LoopDirective::ExecuteTools(tools) => StateTransition::ExecuteTools(tools),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::state::ToolCall;

    #[test]
    fn halt_maps_to_halt_transition() {
        let transition = decide_transition_from_directive(LoopDirective::HaltAndAwaitInput(None));
        assert!(matches!(transition, StateTransition::Halt));
    }

    #[test]
    fn execute_tools_maps_to_execute_tools_transition() {
        let tools = vec![ToolCall {
            name: "memory:stage".to_string(),
            args: serde_json::json!({}),
            id: None,
        }];
        let transition = decide_transition_from_directive(LoopDirective::ExecuteTools(tools));
        assert!(matches!(transition, StateTransition::ExecuteTools(_)));
    }
}
