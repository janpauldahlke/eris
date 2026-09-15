//! Benchmark system for model capability testing.

pub mod harness;
pub mod isolation;
pub mod metrics;
pub mod mutation_tracker;
pub mod reporter;
pub mod routing_offer_fixtures;
pub mod runner;
pub mod scenarios;
pub mod speed_probe;
pub mod storage;
pub mod suite;

pub use harness::BenchmarkHarness;
pub use isolation::{
    BenchmarkIsolation, CleanupReport, IsolationMode, SideEffectFilter, ToolRiskLevel,
};
pub use metrics::{
    BenchmarkReport, CleanupConfirmation, FailureAnalysis, FailureType, QualityMetrics,
    SpeedMetrics, StepTiming, SuiteSpeedAggregate,
};
pub use mutation_tracker::{CleanupGuard, MutationTracker, VaultWriteRecord};
pub use reporter::ReportGenerator;
pub use routing_offer_fixtures::{
    RoutingOfferFixture, all_routing_offer_fixtures, eval_routing_offer_fixture,
    run_all_routing_offer_fixtures,
};
pub use runner::run_benchmark;
pub use storage::BenchmarkStorage;
pub use suite::{
    CleanupAction, CleanupStep, Scenario, ScenarioResult, ScenarioSuite, Step, SuccessCriteria,
    SuiteRegistry,
};
