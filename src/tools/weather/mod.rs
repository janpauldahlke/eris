//! Open-Meteo weather tools: geocode then forecast (two HTTP calls via [`crate::util::ApiHttpClient`]).

mod open_meteo;

pub mod current;
pub mod forecast;

pub use current::WeatherCurrentTool;
pub use forecast::WeatherForecastTool;

#[cfg(test)]
mod integration_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::util::ApiHttpClient;
    use crate::config::{ApiProfile, AppConfig};
    use crate::tools::traits::Tool;

    use super::{WeatherCurrentTool, WeatherForecastTool};

    #[tokio::test]
    async fn weather_current_geocode_then_forecast_wiremock() {
        let geo = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"results":[{"name":"Testville","latitude":1.5,"longitude":2.5,"country":"TS"}]}"#,
            ))
            .mount(&geo)
            .await;

        let fc = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/forecast"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"latitude":1.5,"longitude":2.5,"current":{"time":"2026-01-01T12:00","temperature_2m":12.0}}"#,
            ))
            .mount(&fc)
            .await;

        let mut cfg = AppConfig::default();
        cfg.apis.insert(
            "open_meteo_geocode".into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{}/v1/search", geo.uri()),
                query: [
                    ("name".into(), "{city}".into()),
                    ("count".into(), "1".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: Some(32_768),
                stale_after_secs: None,
            },
        );
        cfg.apis.insert(
            "open_meteo_forecast_current".into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{}/v1/forecast", fc.uri()),
                query: [
                    ("latitude".into(), "{lat}".into()),
                    ("longitude".into(), "{lon}".into()),
                    (
                        "current".into(),
                        "temperature_2m,weather_code".into(),
                    ),
                    ("timezone".into(), "auto".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: None,
                stale_after_secs: None,
            },
        );

        let api = Arc::new(ApiHttpClient::new(Arc::new(cfg)).expect("client"));
        let tool = WeatherCurrentTool { api };
        let out = tool
            .execute(json!({ "city": "Testville" }))
            .await
            .expect("execute");
        assert!(out.contains("Testville"));
        assert!(out.contains("temperature_2m"));
        assert!(out.contains("weather:current"));
    }

    #[tokio::test]
    async fn weather_forecast_geocode_then_hourly_wiremock() {
        let geo = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"results":[{"name":"Testville","latitude":-33.0,"longitude":151.0,"country":"AU"}]}"#,
            ))
            .mount(&geo)
            .await;

        let fc = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/forecast"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"hourly":{"time":["2026-01-01T00:00"],"temperature_2m":[20.0]}}"#,
            ))
            .mount(&fc)
            .await;

        let mut cfg = AppConfig::default();
        cfg.apis.insert(
            "open_meteo_geocode".into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{}/v1/search", geo.uri()),
                query: [
                    ("name".into(), "{city}".into()),
                    ("count".into(), "1".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: Some(32_768),
                stale_after_secs: None,
            },
        );
        cfg.apis.insert(
            "open_meteo_forecast_hourly".into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{}/v1/forecast", fc.uri()),
                query: [
                    ("latitude".into(), "{lat}".into()),
                    ("longitude".into(), "{lon}".into()),
                    ("hourly".into(), "temperature_2m".into()),
                    ("forecast_days".into(), "1".into()),
                    ("timezone".into(), "auto".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: None,
                stale_after_secs: None,
            },
        );

        let api = Arc::new(ApiHttpClient::new(Arc::new(cfg)).expect("client"));
        let tool = WeatherForecastTool { api };
        let out = tool
            .execute(json!({ "city": "Testville" }))
            .await
            .expect("execute");
        assert!(out.contains("hourly"));
        assert!(out.contains("weather:forecast"));
    }
}
