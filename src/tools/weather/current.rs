use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::util::ApiHttpClient;
use crate::executive::error::{FcpError, Result};
use crate::tools::context_view_hint::{ToolContextViewHint, API_TOOL_SNIPPET_CHARS};
use crate::tools::traits::Tool;
use crate::tools::weather::open_meteo::{self, HINT_CURRENT, PROFILE_FORECAST_CURRENT};

#[derive(Deserialize, JsonSchema)]
pub struct WeatherCityArgs {
    /// City or place name to resolve via Open-Meteo geocoding (e.g. "Hamburg", "London").
    pub city: String,
    /// Optional ISO-3166 alpha-2 country code to narrow ambiguous names (e.g. "DE").
    #[serde(default)]
    pub country_code: Option<String>,
}

pub struct WeatherCurrentTool {
    pub api: Arc<ApiHttpClient>,
}

#[async_trait]
impl Tool for WeatherCurrentTool {
    fn name(&self) -> &'static str {
        "weather:current"
    }

    fn description(&self) -> &'static str {
        "Current weather at a place: geocodes the city, then returns Open-Meteo instant (`current`) variables as JSON (temperature, weather_code, humidity). Pass `country_code` if the city name is ambiguous."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(WeatherCityArgs)
    }

    fn context_view_hint(&self) -> ToolContextViewHint {
        ToolContextViewHint::Snippet {
            max_chars: API_TOOL_SNIPPET_CHARS,
        }
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
            "weather:current",
            city,
            cc,
            PROFILE_FORECAST_CURRENT,
            HINT_CURRENT,
        )
        .await
    }
}
