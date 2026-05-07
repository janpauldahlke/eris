//! List calendar events in a time range (`events.list`).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Local, NaiveTime};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::generated::gws_types::calendar::Events;
use crate::tools::calendar::common::format_event_one_line;
use crate::tools::context_view_hint::{API_TOOL_SNIPPET_CHARS, ToolContextViewHint};
use crate::tools::traits::Tool;
use crate::util::CalendarClient;

const DEFAULT_MAX: u32 = 25;
const CAP_MAX: u32 = 250;

#[derive(Deserialize, JsonSchema)]
pub struct CalendarListArgs {
    /// Calendar id (`primary` for the impersonated user’s main calendar). Default: `primary`.
    #[serde(default)]
    pub calendar_id: Option<String>,
    /// RFC3339 lower bound with explicit offset (e.g. `2026-04-15T00:00:00+02:00`). If the user omits the year, use `[SESSION_REFERENCE_TIME]` in the system prompt. If both `time_min` and `time_max` are omitted, uses **today** local midnight → next midnight.
    #[serde(default)]
    pub time_min: Option<String>,
    /// RFC3339 upper bound (exclusive), with explicit offset. Same year anchor as `time_min` when `[SESSION_REFERENCE_TIME]` is present.
    #[serde(default)]
    pub time_max: Option<String>,
    /// Maximum events (default 25, max 250).
    #[serde(default)]
    pub max_results: Option<u32>,
}

pub struct CalendarListTool {
    pub client: Arc<CalendarClient>,
}

fn default_local_day_bounds() -> Result<(String, String)> {
    let d = Local::now().date_naive();
    let t0 = NaiveTime::from_hms_opt(0, 0, 0)
        .ok_or_else(|| FcpError::Config("internal error constructing midnight".into()))?;
    let naive_day = d.and_time(t0);
    let start = match naive_day.and_local_timezone(Local) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(earliest, _) => earliest,
        chrono::LocalResult::None => Local::now(),
    };
    let end = start + chrono::Duration::days(1);
    Ok((start.to_rfc3339(), end.to_rfc3339()))
}

#[async_trait]
impl Tool for CalendarListTool {
    fn name(&self) -> &'static str {
        "calendar:list"
    }

    fn description(&self) -> &'static str {
        "List Google Calendar events in a time window (default: local today). Returns one line per event (id, title, start, end). Use calendar:get for full JSON of one event. When this tool is offered, the system prompt includes `[SESSION_REFERENCE_TIME]`—use it to fill RFC3339 `time_min`/`time_max` if the user gives a date without a year."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(CalendarListArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: CalendarListArgs =
            serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let cal = parsed
            .calendar_id
            .clone()
            .unwrap_or_else(|| "primary".to_string());
        let max = parsed.max_results.unwrap_or(DEFAULT_MAX).min(CAP_MAX);

        let (time_min, time_max) = match (parsed.time_min.clone(), parsed.time_max.clone()) {
            (None, None) => default_local_day_bounds()?,
            (Some(a), None) => {
                let end = chrono::DateTime::parse_from_rfc3339(&a)
                    .map_err(|e| {
                        FcpError::SchemaViolation(format!(
                            "time_min must be RFC3339 if time_max is omitted: {e}"
                        ))
                    })?
                    .with_timezone(&Local)
                    + chrono::Duration::days(1);
                (a, end.to_rfc3339())
            }
            (None, Some(_)) => {
                return Err(FcpError::SchemaViolation(
                    "calendar:list requires time_min when time_max is set, or omit both for today (local)."
                        .into(),
                ));
            }
            (Some(a), Some(b)) => (a, b),
        };

        let raw = self
            .client
            .list_events(
                &cal,
                Some(time_min.as_str()),
                Some(time_max.as_str()),
                max,
                "calendar:list",
            )
            .await?;

        let list: Events = serde_json::from_str(&raw).map_err(|e| {
            tracing::warn!(error = %e, "failed to parse Calendar events.list response");
            FcpError::ToolFault {
                tool_name: "calendar:list".into(),
                reason: "unexpected Google Calendar API response format".into(),
            }
        })?;

        let items = list.items.as_deref().unwrap_or(&[]);
        if items.is_empty() {
            return Ok(format!(
                "[calendar:list] No events between {time_min} and {time_max} (calendar={cal})."
            ));
        }

        let mut out = format!(
            "[calendar:list] calendar={cal} | {} events (start–end window):\n\n",
            items.len()
        );
        for ev in items {
            out.push_str(&format_event_one_line(ev));
            out.push('\n');
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_list_args_schema_is_valid() {
        let schema = schemars::schema_for!(CalendarListArgs);
        let json = serde_json::to_value(&schema).expect("schema json");
        assert!(json.get("properties").is_some() || json.get("$ref").is_some());
    }
}
