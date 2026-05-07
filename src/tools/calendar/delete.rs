//! Delete a Google Calendar event (`events.delete`).

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::util::CalendarClient;

#[derive(Deserialize, JsonSchema)]
pub struct CalendarDeleteArgs {
    #[serde(default)]
    pub calendar_id: Option<String>,
    pub event_id: String,
}

pub struct CalendarDeleteTool {
    pub client: Arc<CalendarClient>,
}

#[async_trait]
impl Tool for CalendarDeleteTool {
    fn name(&self) -> &'static str {
        "calendar:delete"
    }

    fn description(&self) -> &'static str {
        "Permanently remove a Google Calendar event by id (use calendar:list to find ids)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(CalendarDeleteArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: CalendarDeleteArgs =
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
        self.client
            .delete_event(&cal, parsed.event_id.trim(), "calendar:delete")
            .await?;
        Ok(format!(
            "[calendar:delete] Removed event id={} from calendar={}.",
            parsed.event_id.trim(),
            cal
        ))
    }
}
