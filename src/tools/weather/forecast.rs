use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::api::ApiHttpClient;
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use crate::tools::weather::open_meteo::{self, HINT_HOURLY, PROFILE_FORECAST_HOURLY};

#[derive(Deserialize, JsonSchema)]
pub struct WeatherCityArgs {
    /// City or place name to resolve via Open-Meteo geocoding (e.g. "Hamburg", "London").
    pub city: String,
    /// Optional ISO-3166 alpha-2 country code to narrow ambiguous names (e.g. "DE").
    #[serde(default)]
    pub country_code: Option<String>,
}

pub struct WeatherForecastTool {
    pub api: Arc<ApiHttpClient>,
}

#[async_trait]
impl Tool for WeatherForecastTool {
    fn name(&self) -> &'static str {
        "weather:forecast"
    }

    fn description(&self) -> &'static str {
        "Hourly temperature forecast for a place: geocodes the city, then returns Open-Meteo `hourly` time series (several days, configurable in API profile). Pass `country_code` if the city name is ambiguous."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(WeatherCityArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parsed: WeatherCityArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let city = parsed.city.trim();
        if city.is_empty() {
            return Err(FcpError::SchemaViolation(
                "city must be a non-empty string".into(),
            ));
        }
        let cc = parsed
            .country_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        open_meteo::run_weather_tool(
            self.api.as_ref(),
            "weather:forecast",
            city,
            cc,
            PROFILE_FORECAST_HOURLY,
            HINT_HOURLY,
        )
        .await
    }
}
