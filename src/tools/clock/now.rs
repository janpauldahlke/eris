use async_trait::async_trait;
use chrono::Local;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::Result;
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct ClockNowArgs {}

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
        let _: ClockNowArgs = serde_json::from_value(args).map_err(crate::executive::error::FcpError::ParseFault)?;
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
