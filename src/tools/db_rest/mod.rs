//! Deutsche Bahn–style connections via the public `v6.db.transport.rest` wrapper (configured in [`crate::config::AppConfig::apis`]).

mod find_connections;
mod open_transport;

pub use find_connections::DbFindConnectionsTool;

#[cfg(test)]
mod integration_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::{ApiProfile, AppConfig};
    use crate::tools::traits::Tool;
    use crate::util::ApiHttpClient;

    use super::DbFindConnectionsTool;

    #[tokio::test]
    async fn find_connections_locations_then_journeys_wiremock() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/locations"))
            .and(query_param("query", "Hamburg"))
            .and(query_param("results", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"type":"stop","id":"8002549","name":"Hamburg Hbf"}]"#,
            ))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/locations"))
            .and(query_param("query", "Berlin"))
            .and(query_param("results", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"type":"stop","id":"8011160","name":"Berlin Hbf"}]"#,
            ))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/journeys"))
            .and(query_param("from", "8002549"))
            .and(query_param("to", "8011160"))
            .and(query_param("departure", "2026-04-15T08:00:00+02:00"))
            .and(query_param("results", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"journeys":[{"legs":[{"origin":{"id":"8002549","name":"Hamburg Hbf"},"destination":{"id":"8011160","name":"Berlin Hbf"},"departure":"2026-04-15T08:36:00+02:00","plannedDeparture":"2026-04-15T08:36:00+02:00","departureDelay":0,"arrival":"2026-04-15T10:22:00+02:00","plannedArrival":"2026-04-15T10:22:00+02:00","arrivalDelay":0,"line":{"name":"ICE 703","mode":"train","product":"nationalExpress"},"direction":"München Hbf","departurePlatform":"14","plannedDeparturePlatform":"14"}]}]}"#,
            ))
            .mount(&server)
            .await;

        let base = server.uri();
        let mut cfg = AppConfig::default();
        cfg.apis.insert(
            super::open_transport::PROFILE_LOCATIONS.into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{base}/locations"),
                query: [
                    ("query".into(), "{query}".into()),
                    ("results".into(), "1".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: Some(65_536),
                stale_after_secs: None,
            },
        );
        cfg.apis.insert(
            super::open_transport::PROFILE_JOURNEYS_DEPARTURE.into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{base}/journeys"),
                query: [
                    ("from".into(), "{from}".into()),
                    ("to".into(), "{to}".into()),
                    ("departure".into(), "{when}".into()),
                    ("results".into(), "3".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: Some(786_432),
                stale_after_secs: None,
            },
        );
        cfg.apis.insert(
            super::open_transport::PROFILE_JOURNEYS_ARRIVAL.into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{base}/journeys"),
                query: [
                    ("from".into(), "{from}".into()),
                    ("to".into(), "{to}".into()),
                    ("arrival".into(), "{when}".into()),
                    ("results".into(), "3".into()),
                ]
                .into_iter()
                .collect(),
                headers: HashMap::new(),
                max_response_bytes: Some(786_432),
                stale_after_secs: None,
            },
        );

        let api = Arc::new(ApiHttpClient::new(Arc::new(cfg)).expect("client"));
        let tool = DbFindConnectionsTool { api };
        let out = tool
            .execute(json!({
                "from": "Hamburg",
                "to": "Berlin",
                "when": "2026-04-15T08:00:00+02:00",
                "time_constraint": "departure"
            }))
            .await
            .expect("execute");
        assert!(out.contains("db:find_connections"));
        assert!(out.contains("8002549"));
        assert!(out.contains("8011160"));
        assert!(out.contains("ICE 703"));
    }
}
