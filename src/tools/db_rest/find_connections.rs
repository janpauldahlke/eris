use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::ToolContextViewHint;
use crate::tools::db_rest::open_transport::{self, validate_when_iso};
use crate::tools::traits::Tool;
use crate::util::ApiHttpClient;

const MAX_JOURNEYS: usize = 3;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeConstraint {
    #[default]
    Departure,
    Arrival,
}

#[derive(Deserialize, JsonSchema)]
pub struct DbFindConnectionsArgs {
    /// Origin: city or station name (resolved via timetable search, first strong match).
    pub from: String,
    /// Destination: city or station name.
    pub to: String,
    /// RFC 3339 / ISO-8601 datetime **with offset**, e.g. `2026-04-15T08:00:00+02:00`.
    pub when: String,
    /// Whether `when` is interpreted as earliest departure (`departure`) or latest arrival (`arrival`).
    #[serde(default)]
    pub time_constraint: TimeConstraint,
}

pub struct DbFindConnectionsTool {
    pub api: Arc<ApiHttpClient>,
}

#[async_trait]
impl Tool for DbFindConnectionsTool {
    fn name(&self) -> &'static str {
        "db:find_connections"
    }

    fn description(&self) -> &'static str {
        "German public transport connections between two named places: resolves each name to a stop, then returns up to three journeys as compact `summary` + `rides` + `transfers` (walking legs folded into transfers). Pass `when` with an explicit timezone offset. When this tool (or Google Calendar tools) are offered, the system prompt includes `[SESSION_REFERENCE_TIME]`—use it as the calendar year anchor if the user omits the year. Data is a third-party mirror of DB-style timetables; results may differ slightly from the official app."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(DbFindConnectionsArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        // Use global `optimize_context_max_tool_snippet_chars` (not 320): folded payload must stay visible in LLM view.
        ToolContextViewHint::Default
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: DbFindConnectionsArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let from = parsed.from.trim();
        let to = parsed.to.trim();
        if from.is_empty() || to.is_empty() {
            return Err(FcpError::SchemaViolation(
                "`from` and `to` must be non-empty strings".into(),
            ));
        }
        let when = parsed.when.trim();
        validate_when_iso(when)?;

        let arrival = matches!(parsed.time_constraint, TimeConstraint::Arrival);
        open_transport::run_find_connections(
            self.api.as_ref(),
            self.name(),
            from,
            to,
            when,
            arrival,
            MAX_JOURNEYS,
        )
        .await
    }
}
