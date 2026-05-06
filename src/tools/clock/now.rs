use async_trait::async_trait;
use chrono::{Local, SecondsFormat};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::Result;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct ClockNowArgs {}

/// Same wall-clock source as [`ClockNowTool::execute`], formatted for the system prompt when
/// `db:find_connections` or Google Calendar tools are offered so the model can anchor bare dates
/// without a separate `clock:now` call.
pub fn session_reference_time_block_for_prompt() -> String {
    let now = Local::now();
    let rfc = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    let ymd = now.format("%Y-%m-%d").to_string();
    let yyyy = now.format("%Y").to_string();
    format!(
        "[SESSION_REFERENCE_TIME]\n\
         Wall clock when this prompt was built: {rfc} (machine local timezone).\n\
         Default calendar year if the user omits the year: {yyyy} (session date {ymd}).\n\
         Use this anchor for:\n\
         - db:find_connections `when` (RFC3339 with explicit offset);\n\
         - calendar:list `time_min` / `time_max` (RFC3339), and calendar:create / calendar:update `start_datetime` / `end_datetime` (RFC3339 with explicit offset).\n\
         You do not need clock:now solely for this year/date anchor when this block is present.\n\
         [/SESSION_REFERENCE_TIME]"
    )
}

pub struct ClockNowTool;

#[async_trait]
impl Tool for ClockNowTool {
    fn name(&self) -> &'static str {
        "clock:now"
    }

    fn description(&self) -> &'static str {
        "Return the current local time. Primary line is HH:MM : DD/MM/YY (24h, 2-digit year); include offset/tz when reporting to the user."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(ClockNowArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _: ClockNowArgs =
            serde_json::from_value(args).map_err(crate::executive::error::FcpError::ParseFault)?;
        let now = Local::now();
        let primary = now.format("%H:%M : %d/%m/%y").to_string();
        let tz = now.format("%Z").to_string();
        Ok(format!(
            "LOCAL_TIME: {} (timezone: {}, offset: {})",
            primary,
            tz,
            now.format("%:z")
        ))
    }
}

#[cfg(test)]
mod session_ref_tests {
    use super::session_reference_time_block_for_prompt;

    #[test]
    fn session_reference_block_has_markers_and_clock_line() {
        let s = session_reference_time_block_for_prompt();
        assert!(s.contains("[SESSION_REFERENCE_TIME]"));
        assert!(s.contains("[/SESSION_REFERENCE_TIME]"));
        assert!(s.contains("Wall clock when this prompt was built:"));
        assert!(s.contains("db:find_connections"));
        assert!(s.contains("calendar:list"));
    }
}
