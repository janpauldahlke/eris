//! Multi-step and conditional benchmark scenarios (struct-based `Step` model).

use crate::benchmark::suite::{
    CleanupAction, CleanupStep, Scenario, Step, SuccessCriteria,
};
use crate::benchmark::IsolationMode;

/// Multi-hop research: system → vault → memory.
pub fn multi_hop_research_chain() -> Scenario {
    Scenario {
        name: "multi_hop_research_chain".to_string(),
        description: "Multi-step research requiring sequential tool calls".to_string(),
        steps: vec![
            Step {
                description: "Check system status first".to_string(),
                user_prompt: "First check the system health status.".to_string(),
                expected_tool_calls: vec!["system:health".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
            Step {
                description: "Read from vault".to_string(),
                user_prompt: "Now read the Identity file from the vault (00_Invariants/Identity.md).".to_string(),
                expected_tool_calls: vec!["vault:read".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
            Step {
                description: "Stage findings to memory".to_string(),
                user_prompt: "Stage a short summary of what you learned about the system and agent."
                    .to_string(),
                expected_tool_calls: vec!["memory:stage".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
        ],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![CleanupStep {
            description: "Remove staged research memories".to_string(),
            action: CleanupAction::RemoveStagedMemories(vec!["research".to_string()]),
        }],
        timeout_seconds: 90,
        isolation_mode: IsolationMode::Strict,
    }
}

/// Memory query chain: stage then query.
pub fn memory_query_chain() -> Scenario {
    Scenario {
        name: "memory_query_chain".to_string(),
        description: "Stage information then query it back".to_string(),
        steps: vec![
            Step {
                description: "Stage related facts".to_string(),
                user_prompt: "Stage these three facts: Apples are red. Bananas are yellow. Grapes are purple."
                    .to_string(),
                expected_tool_calls: vec!["memory:stage".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 5,
            },
            Step {
                description: "Query for fruit colors".to_string(),
                user_prompt: "Query memories for information about fruit colors.".to_string(),
                expected_tool_calls: vec!["memory:query".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
        ],
        success_criteria: SuccessCriteria::ResponseContains("red".to_string()),
        cleanup: vec![CleanupStep {
            description: "Remove fruit staging keys".to_string(),
            action: CleanupAction::RemoveStagedMemories(vec![
                "apple".to_string(),
                "banana".to_string(),
                "grape".to_string(),
            ]),
        }],
        timeout_seconds: 90,
        isolation_mode: IsolationMode::Strict,
    }
}

/// Time then system check.
pub fn conditional_weather_check() -> Scenario {
    Scenario {
        name: "conditional_weather_check".to_string(),
        description: "Test conditional reasoning based on tool results".to_string(),
        steps: vec![
            Step {
                description: "Get current time".to_string(),
                user_prompt: "What time is it now? Use the clock tool.".to_string(),
                expected_tool_calls: vec!["clock:now".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
            Step {
                description: "Check system health".to_string(),
                user_prompt:
                    "Check system health and say whether services look OK or if there is an issue."
                        .to_string(),
                expected_tool_calls: vec!["system:health".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
        ],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![],
        timeout_seconds: 60,
        isolation_mode: IsolationMode::Strict,
    }
}

pub fn error_recovery_simulation() -> Scenario {
    Scenario {
        name: "error_recovery_simulation".to_string(),
        description: "Attempt invalid read then recover with valid path".to_string(),
        steps: vec![
            Step {
                description: "Try missing file".to_string(),
                user_prompt: "Try to read NonExistentFile.md from the vault.".to_string(),
                expected_tool_calls: vec!["vault:read".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 2,
            },
            Step {
                description: "Read Identity".to_string(),
                user_prompt: "Now read 00_Invariants/Identity.md.".to_string(),
                expected_tool_calls: vec!["vault:read".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
        ],
        success_criteria: SuccessCriteria::ResponseContains("eris".to_string()),
        cleanup: vec![],
        timeout_seconds: 90,
        isolation_mode: IsolationMode::Relaxed,
    }
}

pub fn conditional_tool_selection() -> Scenario {
    Scenario {
        name: "conditional_tool_selection".to_string(),
        description: "List then search the vault".to_string(),
        steps: vec![
            Step {
                description: "List directory".to_string(),
                user_prompt: "List files under 00_Invariants.".to_string(),
                expected_tool_calls: vec!["vault:list".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
            Step {
                description: "Search vault".to_string(),
                user_prompt: "Search the vault for mentions of Eris or Agent.".to_string(),
                expected_tool_calls: vec!["vault:search".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
        ],
        success_criteria: SuccessCriteria::AllToolsCalled,
        cleanup: vec![],
        timeout_seconds: 90,
        isolation_mode: IsolationMode::Strict,
    }
}

pub fn multi_hop_with_branching() -> Scenario {
    Scenario {
        name: "multi_hop_with_branching".to_string(),
        description: "Clock then vault read or search".to_string(),
        steps: vec![
            Step {
                description: "Get time".to_string(),
                user_prompt: "What time is it?".to_string(),
                expected_tool_calls: vec!["clock:now".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 3,
            },
            Step {
                description: "Find agent info".to_string(),
                user_prompt: "Find information about the agent — read Identity.md or search the vault."
                    .to_string(),
                expected_tool_calls: vec!["vault:read".to_string(), "vault:search".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 4,
            },
        ],
        success_criteria: SuccessCriteria::AnyToolCalled,
        cleanup: vec![],
        timeout_seconds: 90,
        isolation_mode: IsolationMode::Strict,
    }
}
