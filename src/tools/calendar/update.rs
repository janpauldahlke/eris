//! Partially update a Google Calendar event (`events.patch`).

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::executive::error::{FcpError, Result};
use crate::generated::gws_types::calendar::Event;
use crate::tools::calendar::common::format_event_one_line;
use crate::tools::context_view_hint::{ToolContextViewHint, API_TOOL_SNIPPET_CHARS};
use crate::tools::traits::Tool;
use crate::util::CalendarClient;

#[derive(Deserialize, JsonSchema)]
pub struct CalendarUpdateArgs {
    #[serde(default)]
    pub calendar_id: Option<String>,
    pub event_id: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub start_datetime: Option<String>,
    #[serde(default)]
    pub end_datetime: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
}

pub struct CalendarUpdateTool {
    pub client: Arc<CalendarClient>,
}

#[async_trait]
impl Tool for CalendarUpdateTool {
    fn name(&self) -> &'static str {
        "calendar:update"
    }

    fn description(&self) -> &'static str {
        "Patch a Google Calendar event: pass event_id and any fields to change (summary, RFC3339 start/end, description, location). When changing times, use explicit offsets; `[SESSION_REFERENCE_TIME]` anchors the year if the user omits it."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(CalendarUpdateArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: CalendarUpdateArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if parsed.event_id.trim().is_empty() {
            return Err(FcpError::SchemaViolation(
                "event_id must be non-empty".into(),
            ));
        }

        let has_patch = parsed.summary.is_some()
            || parsed.start_datetime.is_some()
            || parsed.end_datetime.is_some()
            || parsed.description.is_some()
            || parsed.location.is_some();
        if !has_patch {
            return Err(FcpError::SchemaViolation(
                "provide at least one of summary, start_datetime, end_datetime, description, location"
                    .into(),
            ));
        }

        let cal = parsed
            .calendar_id
            .clone()
            .unwrap_or_else(|| "primary".to_string());

        let mut body = json!({});
        if let Some(ref s) = parsed.summary {
            if !s.trim().is_empty() {
                body["summary"] = json!(s.trim());
            }
        }
        if let Some(ref d) = parsed.description {
            body["description"] = json!(d);
        }
        if let Some(ref loc) = parsed.location {
            if !loc.trim().is_empty() {
                body["location"] = json!(loc);
            }
        }

        if parsed.start_datetime.is_some() || parsed.end_datetime.is_some() {
            let (Some(st), Some(en)) = (
                parsed.start_datetime.as_deref(),
                parsed.end_datetime.as_deref(),
            ) else {
                return Err(FcpError::SchemaViolation(
                    "calendar:update requires both start_datetime and end_datetime when changing times"
                        .into(),
                ));
            };
            let mut start = json!({ "dateTime": st });
            let mut end = json!({ "dateTime": en });
            if let Some(ref tz) = parsed.time_zone {
                if !tz.trim().is_empty() {
                    start["timeZone"] = json!(tz);
                    end["timeZone"] = json!(tz);
                }
            }
            body["start"] = start;
            body["end"] = end;
        }

        let raw = self
            .client
            .patch_event(
                &cal,
                parsed.event_id.trim(),
                &body,
                "calendar:update",
            )
            .await?;

        let ev: Event = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "calendar:update parse");
            FcpError::ToolFault {
                tool_name: "calendar:update".into(),
                reason: "unexpected Calendar API response after patch".into(),
            }
        })?;

        Ok(format!(
            "[calendar:update] Updated: {}",
            format_event_one_line(&ev)
        ))
    }
}
