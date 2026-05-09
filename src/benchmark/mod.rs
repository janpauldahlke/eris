//! Benchmark system for model capability testing.

pub mod harness;
pub mod isolation;
pub mod metrics;
pub mod mutation_tracker;
pub mod reporter;
pub mod runner;
pub mod scenarios;
pub mod speed_probe;
pub mod storage;
pub mod suite;

pub use harness::BenchmarkHarness;
pub use isolation::{BenchmarkIsolation, CleanupReport, IsolationMode, SideEffectFilter, ToolRiskLevel};
pub use metrics::{BenchmarkReport, CleanupConfirmation, FailureAnalysis, FailureType, QualityMetrics, SpeedMetrics};
pub use mutation_tracker::{CleanupGuard, MutationTracker, VaultWriteRecord};
pub use reporter::ReportGenerator;
pub use runner::run_benchmark;
pub use storage::BenchmarkStorage;
pub use suite::{
    CleanupAction, CleanupStep, Scenario, ScenarioResult, ScenarioSuite, Step, SuccessCriteria,
    SuiteRegistry,
};
