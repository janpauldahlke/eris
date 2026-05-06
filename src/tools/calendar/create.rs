//! Create a Google Calendar event (`events.insert`).

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::generated::gws_types::calendar::Event;
use crate::tools::calendar::common::format_event_one_line;
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::util::CalendarClient;

#[derive(Deserialize, JsonSchema)]
pub struct CalendarCreateArgs {
    #[serde(default)]
    pub calendar_id: Option<String>,
    pub summary: String,
    /// RFC3339 start with explicit offset (e.g. `2026-04-15T14:00:00+02:00`). Year from `[SESSION_REFERENCE_TIME]` when the user omits it.
    pub start_datetime: String,
    /// RFC3339 end (must be after start); include explicit offset.
    pub end_datetime: String,
    /// IANA zone (e.g. `Europe/Berlin`); helps all-day edge cases when using date-only elsewhere.
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
}

pub struct CalendarCreateTool {
    pub client: Arc<CalendarClient>,
}

#[async_trait]
impl Tool for CalendarCreateTool {
    fn name(&self) -> &'static str {
        "calendar:create"
    }

    fn description(&self) -> &'static str {
        "Create a Google Calendar event with title, RFC3339 start/end (explicit offset), optional description and location. When offered, `[SESSION_REFERENCE_TIME]` in the system prompt anchors the year for bare dates."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(CalendarCreateArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: CalendarCreateArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if parsed.summary.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "summary must be non-empty".into(),
            ));
        }
        let cal = parsed
            .calendar_id
            .clone()
            .unwrap_or_else(|| "primary".to_string());

        let mut start = json!({ "dateTime": parsed.start_datetime });
        let mut end = json!({ "dateTime": parsed.end_datetime });
        if let Some(ref tz) = parsed.time_zone {
            if !tz.trim().is_empty() {
                start["timeZone"] = json!(tz);
                end["timeZone"] = json!(tz);
            }
        }

        let mut body = json!({
            "summary": parsed.summary.trim(),
            "start": start,
            "end": end,
        });
        if let Some(ref d) = parsed.description {
            if !d.trim().is_empty() {
                body["description"] = json!(d);
            }
        }
        if let Some(ref loc) = parsed.location {
            if !loc.trim().is_empty() {
                body["location"] = json!(loc);
            }
        }

        let raw = self
            .client
            .insert_event(&cal, &body, "calendar:create")
            .await?;
        let ev: Event = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "calendar:create parse");
            FcpError::ToolFault {
                tool_name: "calendar:create".into(),
                reason: "unexpected Calendar API response after insert".into(),
            }
        })?;

        Ok(format!(
            "[calendar:create] Created: {}\nRaw id: {}",
            format_event_one_line(&ev),
            ev.id.as_deref().unwrap_or("?")
        ))
    }
}
