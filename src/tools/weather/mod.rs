//! Open-Meteo weather tools: geocode then forecast (two HTTP calls via [`crate::util::ApiHttpClient`]).

mod open_meteo;
pub mod report;

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

    use crate::config::{ApiProfile, AppConfig};
    use crate::tools::traits::Tool;
    use crate::util::ApiHttpClient;

    use super::{WeatherCurrentTool, WeatherForecastTool};

    #[tokio::test(flavor = "current_thread")]
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
                r#"{"timezone":"Europe/Berlin","timezone_abbreviation":"CET","current":{"time":"2026-01-01T12:00","interval":900,"temperature_2m":12.0,"weather_code":0,"relative_humidity_2m":50,"precipitation":0.0,"cloud_cover":10}}"#,
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
                    ("current".into(), "temperature_2m,weather_code".into()),
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
        assert!(out.contains("\"report\""));
        assert!(out.contains("## 🌡️ Now"));
        assert!(out.contains("weather:current"));
        assert!(!out.contains("\"forecast\""));
    }

    #[tokio::test(flavor = "current_thread")]
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
                r#"{"timezone":"Australia/Sydney","current":{"time":"2026-01-01T12:00"},"hourly":{"time":["2026-01-01T12:00","2026-01-01T13:00","2026-01-01T14:00"],"temperature_2m":[20.0,21.0,22.0],"precipitation":[0.0,0.0,0.0],"weather_code":[0,0,0],"is_day":[1,1,1]},"daily":{"time":["2026-01-01"],"temperature_2m_min":[18.0],"temperature_2m_max":[22.0],"precipitation_sum":[0.0],"weather_code":[0]}}"#,
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
                    ("current".into(), "temperature_2m,weather_code".into()),
                    ("hourly".into(), "temperature_2m,precipitation,weather_code,is_day".into()),
                    ("daily".into(), "temperature_2m_max,temperature_2m_min,precipitation_sum,weather_code".into()),
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
        assert!(out.contains("\"report\""));
        assert!(out.contains("## 🌤️ Forecast"));
        assert!(out.contains("### 📅 Next few days"));
        assert!(out.contains("weather:forecast"));
        assert!(!out.contains("\"forecast\":"));
    }
}
