//! Benchmark scenario definitions and suite registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use crate::benchmark::metrics::ScenarioResult;

/// A single step in a benchmark scenario.
#[derive(Clone)]
pub struct Step {
    pub description: String,
    pub user_prompt: String,
    pub expected_tool_calls: Vec<String>,
    pub arg_validator: Option<fn(&serde_json::Value) -> bool>,
    pub content_validator: Option<fn(&str) -> bool>,
    pub max_rounds: u32,
}

// Manual Serialize/Deserialize for Step (function pointers can't be serialized)
impl Serialize for Step {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Step", 4)?;
        state.serialize_field("description", &self.description)?;
        state.serialize_field("user_prompt", &self.user_prompt)?;
        state.serialize_field("expected_tool_calls", &self.expected_tool_calls)?;
        state.serialize_field("max_rounds", &self.max_rounds)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StepHelper {
            description: String,
            user_prompt: String,
            expected_tool_calls: Vec<String>,
            max_rounds: u32,
        }

        let helper = StepHelper::deserialize(deserializer)?;
        Ok(Step {
            description: helper.description,
            user_prompt: helper.user_prompt,
            expected_tool_calls: helper.expected_tool_calls,
            arg_validator: None,
            content_validator: None,
            max_rounds: helper.max_rounds,
        })
    }
}

/// Criteria for determining if a scenario succeeded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SuccessCriteria {
    /// All expected tool calls must be made.
    AllToolsCalled,
    /// At least one of the expected tools must be called.
    AnyToolCalled,
    /// Custom validation function (not serializable, will default to None on load).
    #[serde(skip)]
    Custom(fn(&ScenarioResult) -> bool),
    /// Response must contain expected content substring.
    ResponseContains(String),
    /// Response must be valid JSON.
    ValidJson,
}

/// Actions to take after scenario completion for cleanup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupStep {
    pub description: String,
    pub action: CleanupAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CleanupAction {
    /// Remove staged memories created during the scenario.
    RemoveStagedMemories(Vec<String>),
    /// Remove ephemeral entries.
    RemoveEphemeralEntries(Vec<String>),
    /// Remove temp files.
    RemoveTempFiles(Vec<std::path::PathBuf>),
}

/// A complete benchmark scenario.
#[derive(Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub steps: Vec<Step>,
    pub success_criteria: SuccessCriteria,
    pub cleanup: Vec<CleanupStep>,
    pub timeout_seconds: u64,
    pub isolation_mode: super::IsolationMode,
}

/// A collection of scenarios forming a benchmark suite.
#[derive(Clone, Serialize, Deserialize)]
pub struct ScenarioSuite {
    pub name: String,
    pub description: String,
    pub scenarios: Vec<Scenario>,
}

impl ScenarioSuite {
    /// Create a new empty suite.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            scenarios: Vec::new(),
        }
    }

    /// Add a scenario to the suite.
    pub fn add_scenario(mut self, scenario: Scenario) -> Self {
        self.scenarios.push(scenario);
        self
    }

    /// Get the number of scenarios in the suite.
    pub fn len(&self) -> usize {
        self.scenarios.len()
    }

    /// Check if the suite is empty.
    pub fn is_empty(&self) -> bool {
        self.scenarios.is_empty()
    }
}

/// Registry of all available benchmark suites.
pub struct SuiteRegistry {
    suites: HashMap<String, ScenarioSuite>,
}

impl Default for SuiteRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SuiteRegistry {
    /// Create a new registry with all predefined suites.
    pub fn new() -> Self {
        let mut registry = Self {
            suites: HashMap::new(),
        };
        registry.register_predefined();
        registry
    }

    /// Get a suite by name.
    pub fn get(&self, name: &str) -> Option<&ScenarioSuite> {
        self.suites.get(name)
    }

    /// List all available suite names.
    pub fn list(&self) -> Vec<&str> {
        let mut names: Vec<_> = self.suites.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Register all predefined suites.
    fn register_predefined(&mut self) {
        self.suites.insert(
            "quick".to_string(),
            Self::build_quick_suite(),
        );
        self.suites.insert(
            "standard".to_string(),
            Self::build_standard_suite(),
        );
        self.suites.insert(
            "comprehensive".to_string(),
            Self::build_comprehensive_suite(),
        );
    }

    /// Build the quick suite (fast sanity checks, ~30 seconds).
    fn build_quick_suite() -> ScenarioSuite {
        ScenarioSuite::new(
            "quick",
            "Fast sanity checks for basic JSON compliance and tool calling (~30s)",
        )
        .add_scenario(super::scenarios::simple::json_protocol_compliance())
        .add_scenario(super::scenarios::simple::single_memory_stage())
        .add_scenario(super::scenarios::simple::vault_read())
        .add_scenario(super::scenarios::simple::system_health_check())
        .add_scenario(super::scenarios::simple::clock_query())
    }

    /// Build the standard suite (core capability tests, ~2-3 minutes).
    fn build_standard_suite() -> ScenarioSuite {
        let mut suite = Self::build_quick_suite();
        suite.name = "standard".to_string();
        suite.description = "Core capability tests including multi-step reasoning (~2-3m)".to_string();
        
        // Add additional scenarios
        suite.scenarios.push(super::scenarios::complex::multi_hop_research_chain());
        suite.scenarios.push(super::scenarios::complex::memory_query_chain());
        suite.scenarios.push(super::scenarios::complex::conditional_weather_check());
        suite.scenarios.push(super::scenarios::adversarial::json_with_noise());
        
        suite
    }

    /// Build the comprehensive suite (deep capability analysis, ~5-10 minutes).
    fn build_comprehensive_suite() -> ScenarioSuite {
        let mut suite = Self::build_standard_suite();
        suite.name = "comprehensive".to_string();
        suite.description = "Deep capability analysis including stress tests (~5-10m)".to_string();
        
        // Add stress test scenarios
        suite.scenarios.push(super::scenarios::adversarial::unicode_handling());
        suite.scenarios.push(super::scenarios::adversarial::deeply_nested_json());
        suite.scenarios.push(super::scenarios::adversarial::large_array_parsing());
        suite.scenarios.push(super::scenarios::complex::error_recovery_simulation());
        suite.scenarios.push(super::scenarios::complex::conditional_tool_selection());
        suite.scenarios.push(super::scenarios::complex::multi_hop_with_branching());
        
        suite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suite_registry() {
        let registry = SuiteRegistry::new();
        assert!(registry.get("quick").is_some());
        assert!(registry.get("standard").is_some());
        assert!(registry.get("comprehensive").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_suite_sizes() {
        let registry = SuiteRegistry::new();
        
        let quick = registry.get("quick").unwrap();
        assert!(!quick.is_empty());
        
        let standard = registry.get("standard").unwrap();
        assert!(standard.len() >= quick.len());
        
        let comprehensive = registry.get("comprehensive").unwrap();
        assert!(comprehensive.len() >= standard.len());
    }

    #[test]
    fn test_scenario_serialization() {
        let scenario = Scenario {
            name: "test".to_string(),
            description: "Test scenario".to_string(),
            steps: vec![Step {
                description: "Step 1".to_string(),
                user_prompt: "Test prompt".to_string(),
                expected_tool_calls: vec!["tool".to_string()],
                arg_validator: None,
                content_validator: None,
                max_rounds: 5,
            }],
            success_criteria: SuccessCriteria::AllToolsCalled,
            cleanup: vec![],
            timeout_seconds: 60,
            isolation_mode: super::super::IsolationMode::Strict,
        };

        let json = serde_json::to_string(&scenario).unwrap();
        let deserialized: Scenario = serde_json::from_str(&json).unwrap();
        
        assert_eq!(scenario.name, deserialized.name);
        assert_eq!(scenario.steps.len(), deserialized.steps.len());
    }
}
