//! Stress-style scenarios (struct-based `Step` model).

use crate::benchmark::suite::{
    CleanupAction, CleanupStep, Scenario, Step, SuccessCriteria,
};
use crate::benchmark::IsolationMode;

pub fn json_with_noise() -> Scenario {
    Scenario {
        name: "json_with_noise".to_string(),
        description: "Verbose system health request".to_string(),
        steps: vec![Step {
            description: "Verbose health check".to_string(),
            user_prompt:
                "Check system health and give a detailed status. Use the system health tool."
                    .to_string(),
            expected_tool_calls: vec!["system:health".to_string()],
            arg_validator: None,
            content_validator: None,
            max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![],
        timeout_seconds: 45,
        isolation_mode: IsolationMode::Strict,
    }
}

pub fn deeply_nested_json() -> Scenario {
    Scenario {
        name: "deeply_nested_json".to_string(),
        description: "Stage structured nested content".to_string(),
        steps: vec![Step {
            description: "Stage nested payload".to_string(),
            user_prompt: "Stage this as memory content: {\"meta\":{\"v\":\"1\"},\"body\":\"benchmark\"}"
                .to_string(),
            expected_tool_calls: vec!["memory:stage".to_string()],
            arg_validator: None,
            content_validator: None,
            max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::ValidJson,
        cleanup: vec![CleanupStep {
            description: "Remove nested staging".to_string(),
            action: CleanupAction::RemoveStagedMemories(vec!["complex".to_string()]),
        }],
        timeout_seconds: 45,
        isolation_mode: IsolationMode::Strict,
    }
}

pub fn large_array_parsing() -> Scenario {
    Scenario {
        name: "large_array_parsing".to_string(),
        description: "List vault root (may be large)".to_string(),
        steps: vec![Step {
            description: "List vault root".to_string(),
            user_prompt: "List files at the vault root directory.".to_string(),
            expected_tool_calls: vec!["vault:list".to_string()],
            arg_validator: None,
            content_validator: None,
            max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![],
        timeout_seconds: 45,
        isolation_mode: IsolationMode::Strict,
    }
}

pub fn unicode_handling() -> Scenario {
    Scenario {
        name: "unicode_handling".to_string(),
        description: "Unicode in staged content".to_string(),
        steps: vec![Step {
            description: "Stage unicode".to_string(),
            user_prompt: "Stage this text: 测试 🎉 café".to_string(),
            expected_tool_calls: vec!["memory:stage".to_string()],
            arg_validator: None,
            content_validator: None,
            max_rounds: 3,
        }],
        success_criteria: SuccessCriteria::ValidJson,
        cleanup: vec![CleanupStep {
            description: "Remove unicode staging".to_string(),
            action: CleanupAction::RemoveStagedMemories(vec!["unicode".to_string()]),
        }],
        timeout_seconds: 45,
        isolation_mode: IsolationMode::Strict,
    }
}
