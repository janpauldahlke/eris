//! Open-Meteo geocoding + forecast sequence (two HTTP calls). URLs come from [`crate::config::ApiProfile`] ids.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};
use crate::util::ApiHttpClient;

pub const PROFILE_GEOCODE: &str = "open_meteo_geocode";
pub const PROFILE_GEOCODE_CC: &str = "open_meteo_geocode_cc";
pub const PROFILE_FORECAST_CURRENT: &str = "open_meteo_forecast_current";
pub const PROFILE_FORECAST_HOURLY: &str = "open_meteo_forecast_hourly";

pub const HINT_CURRENT: &str = "Open-Meteo `current` block: use every field the JSON provides. Always state temperature (temperature_2m, °C). When present, also summarize precipitation/rain (e.g. precipitation, rain, showers in mm) and sun-related conditions (e.g. cloud_cover %, shortwave_radiation, sunshine_duration). Interpret weather_code (WMO) for sky/conditions; mention relative_humidity_2m when present.";
pub const HINT_HOURLY: &str = "Open-Meteo `hourly` block: arrays align by index (same length). Always cover temperature (temperature_2m). When present, also describe precipitation or rain over the window and cloud/sun-related series (e.g. cloud_cover, precipitation_probability, shortwave_radiation). `forecast_days` limits horizon.";

#[derive(Deserialize)]
struct GeocodeResponse {
    results: Option<Vec<GeocodeHit>>,
}

#[derive(Deserialize)]
struct GeocodeHit {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
}

pub fn map_api_err(tool_name: &'static str, e: FcpError) -> FcpError {
    match e {
        FcpError::ToolFault {
            tool_name: tn,
            reason,
        } if tn == "api_client" => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason,
        },
        FcpError::NetworkFault(_) => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "weather data service unreachable".into(),
        },
        FcpError::Config(msg) => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: format!("weather configuration: {msg}"),
        },
        other => other,
    }
}

/// Geocode city name, then fetch forecast JSON; wrap in a stable envelope for the LLM.
pub async fn run_weather_tool(
    api: &ApiHttpClient,
    tool_name: &'static str,
    city: &str,
    country_code: Option<&str>,
    forecast_profile: &'static str,
    hint: &'static str,
) -> Result<String> {
    let (label, lat, lon) = resolve_location(api, tool_name, city, country_code).await?;
    let body = fetch_forecast_raw(api, tool_name, lat, lon, forecast_profile).await?;
    let forecast: Value = serde_json::from_str(&body).map_err(|e| FcpError::ToolFault {
        tool_name: tool_name.to_string(),
        reason: format!("forecast JSON parse error: {e}"),
    })?;
    let envelope = json!({
        "tool": tool_name,
        "location": label,
        "latitude": lat,
        "longitude": lon,
        "hint": hint,
        "forecast": forecast,
    });
    serde_json::to_string(&envelope).map_err(FcpError::ParseFault)
}

async fn resolve_location(
    api: &ApiHttpClient,
    tool_name: &'static str,
    city: &str,
    country_code: Option<&str>,
) -> Result<(String, f64, f64)> {
    let mut params = HashMap::new();
    params.insert("city".into(), city.to_string());
    let geo_json = if let Some(cc) = country_code.filter(|s| !s.trim().is_empty()) {
        params.insert("country_code".into(), cc.trim().to_uppercase());
        api.get_templated(PROFILE_GEOCODE_CC, &params)
            .await
            .map_err(|e| map_api_err(tool_name, e))?
    } else {
        api.get_templated(PROFILE_GEOCODE, &params)
            .await
            .map_err(|e| map_api_err(tool_name, e))?
    };
    parse_geocode_first(tool_name, &geo_json)
}

fn parse_geocode_first(tool_name: &'static str, json: &str) -> Result<(String, f64, f64)> {
    let parsed: GeocodeResponse = serde_json::from_str(json).map_err(|e| FcpError::ToolFault {
        tool_name: tool_name.to_string(),
        reason: format!("geocode response parse error: {e}"),
    })?;
    let hit = parsed
        .results
        .as_ref()
        .and_then(|r| r.first())
        .ok_or_else(|| FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "no matching location found for that city".into(),
        })?;
    let label = match &hit.country {
        Some(c) => format!("{}, {}", hit.name, c),
        None => hit.name.clone(),
    };
    Ok((label, hit.latitude, hit.longitude))
}

async fn fetch_forecast_raw(
    api: &ApiHttpClient,
    tool_name: &'static str,
    lat: f64,
    lon: f64,
    forecast_profile: &str,
) -> Result<String> {
    let mut params = HashMap::new();
    params.insert("lat".into(), format!("{lat}"));
    params.insert("lon".into(), format!("{lon}"));
    api.get_templated(forecast_profile, &params)
        .await
        .map_err(|e| map_api_err(tool_name, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_geocode_first_ok() {
        let j = r#"{"results":[{"name":"Berlin","latitude":52.52,"longitude":13.41,"country":"Germany"}]}"#;
        let (label, lat, lon) = parse_geocode_first("weather:current", j).expect("ok");
        assert_eq!(label, "Berlin, Germany");
        assert!((lat - 52.52).abs() < 0.01);
        assert!((lon - 13.41).abs() < 0.01);
    }

    #[test]
    fn parse_geocode_empty_results() {
        let j = r#"{"results":[]}"#;
        let r = parse_geocode_first("weather:current", j);
        assert!(matches!(r, Err(FcpError::ToolFault { .. })));
    }
}
