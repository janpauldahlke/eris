use crate::executive::error::Result;
use async_trait::async_trait;
use schemars::schema::RootSchema;
use serde_json::Value;

use super::context_view_hint::ToolContextViewHint;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> RootSchema;
    async fn execute(&self, args: Value) -> Result<String>;

    /// How to present this tool’s success line in the LLM-only context view. Default: global snippet cap.
    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Default
    }

    /// If `true`, the orchestrator will NOT suppress duplicate calls to this
    /// tool within a single `step()`.  Override for polling/refresh tools
    /// whose response changes between invocations even with identical args
    /// (e.g. dashboards, feeds, notification inboxes).
    fn allow_repeat_in_turn(&self) -> bool {
        false
    }
}
