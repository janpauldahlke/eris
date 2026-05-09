use crate::benchmark::suite::{CleanupAction, CleanupStep, Scenario, Step, SuccessCriteria};
use crate::benchmark::IsolationMode;

pub fn json_protocol_compliance() -> Scenario {
    Scenario {
        name: "json_protocol_compliance".to_string(),
        description: "Verify the model can produce valid JSON tool calls".to_string(),
        steps: vec![Step {
            description: "Request system health check in JSON".to_string(),
            user_prompt: "Check the system health and report the status. Use the system:health tool.".to_string(),
            expected_tool_calls: vec!["system:health".to_string()],
            arg_validator: None, content_validator: None, max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![], timeout_seconds: 30, isolation_mode: IsolationMode::Strict,
    }
}

pub fn single_memory_stage() -> Scenario {
    Scenario {
        name: "single_memory_stage".to_string(),
        description: "Test basic memory staging functionality".to_string(),
        steps: vec![Step {
            description: "Stage a test memory entry".to_string(),
            user_prompt: "Please stage this information for later: 'The benchmark is running on test suite version 1.0'.".to_string(),
            expected_tool_calls: vec!["memory:stage".to_string()],
            arg_validator: None, content_validator: None, max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![CleanupStep { description: "Remove staged test memories".to_string(), action: CleanupAction::RemoveStagedMemories(vec!["benchmark".to_string()]) }],
        timeout_seconds: 30, isolation_mode: IsolationMode::Strict,
    }
}

pub fn vault_read() -> Scenario {
    Scenario {
        name: "vault_read".to_string(),
        description: "Test vault file reading".to_string(),
        steps: vec![Step {
            description: "Read Identity.md from vault".to_string(),
            user_prompt: "Read the file 00_Invariants/Identity.md from the vault and tell me what the agent name is.".to_string(),
            expected_tool_calls: vec!["vault:read".to_string()],
            arg_validator: None, content_validator: None, max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled, cleanup: vec![], timeout_seconds: 30, isolation_mode: IsolationMode::Strict,
    }
}

pub fn system_health_check() -> Scenario {
    Scenario {
        name: "system_health_check".to_string(),
        description: "Test system health tool access".to_string(),
        steps: vec![Step {
            description: "Check system status".to_string(),
            user_prompt: "Check the system status using the system health tool.".to_string(),
            expected_tool_calls: vec!["system:health".to_string()],
            arg_validator: None, content_validator: None, max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled, cleanup: vec![], timeout_seconds: 30, isolation_mode: IsolationMode::Strict,
    }
}

pub fn clock_query() -> Scenario {
    Scenario {
        name: "clock_query".to_string(),
        description: "Test clock tool for current time".to_string(),
        steps: vec![Step {
            description: "Get current time".to_string(),
            user_prompt: "What is the current time? Use the clock tool.".to_string(),
            expected_tool_calls: vec!["clock:now".to_string()],
            arg_validator: None, content_validator: None, max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled, cleanup: vec![], timeout_seconds: 30, isolation_mode: IsolationMode::Strict,
    }
}
