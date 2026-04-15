//! Fetch one Google Calendar event by id (`events.get`).

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::generated::gws_types::calendar::Event;
use crate::tools::calendar::common::format_event_one_line;
use crate::tools::context_view_hint::{ToolContextViewHint, API_TOOL_SNIPPET_CHARS};
use crate::tools::traits::Tool;
use crate::util::CalendarClient;

#[derive(Deserialize, JsonSchema)]
pub struct CalendarGetArgs {
    /// Calendar id; default `primary`.
    #[serde(default)]
    pub calendar_id: Option<String>,
    /// Event id from calendar:list or the Calendar UI.
    pub event_id: String,
}

pub struct CalendarGetTool {
    pub client: Arc<CalendarClient>,
}

#[async_trait]
impl Tool for CalendarGetTool {
    fn name(&self) -> &'static str {
        "calendar:get"
    }

    fn description(&self) -> &'static str {
        "Read one Google Calendar event by id. Returns a summary line plus pretty-printed JSON (start, end, attendees, conference, etc.)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(CalendarGetArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: CalendarGetArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if parsed.event_id.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "event_id must be non-empty".into(),
            ));
        }
        let cal = parsed
            .calendar_id
            .clone()
            .unwrap_or_else(|| "primary".to_string());
        let raw = self
            .client
            .get_event(&cal, parsed.event_id.trim(), "calendar:get")
            .await?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "calendar:get JSON parse");
            FcpError::ToolFault {
                tool_name: "calendar:get".into(),
                reason: "unexpected Calendar API response".into(),
            }
        })?;
        let pretty = serde_json::to_string_pretty(&v).map_err(|e| {
            FcpError::ToolFault {
                tool_name: "calendar:get".into(),
                reason: format!("serialize event: {e}"),
            }
        })?;

        let summary_line = match serde_json::from_value::<Event>(v.clone()) {
            Ok(ev) => format_event_one_line(&ev),
            Err(_) => String::new(),
        };

        Ok(format!("[calendar:get]\n{summary_line}\n\n{pretty}"))
    }
}
