//! English Wikipedia summary tool (`wiki:summary`).

mod summary;

pub use summary::WikiSummaryTool;

#[cfg(test)]
mod integration_tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::{ApiProfile, AppConfig};
    use crate::tools::traits::Tool;
    use crate::util::ApiHttpClient;

    use super::WikiSummaryTool;

    #[tokio::test]
    async fn wiki_summary_wiremock_path_and_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/rest_v1/page/summary/Earth"))
            .and(header(
                "user-agent",
                "Eris-Agent/1.0 (Local autonomous system)",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"title":"Earth","extract":"Planet.","description":"Planet","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Earth"}}}"#,
            ))
            .mount(&server)
            .await;

        let mut cfg = AppConfig::default();
        cfg.apis.insert(
            "wikipedia_page_summary".into(),
            ApiProfile {
                enabled: true,
                base_url: format!("{}/api/rest_v1/page/summary/{{title}}", server.uri()),
                query: HashMap::new(),
                headers: [(
                    "User-Agent".into(),
                    "Eris-Agent/1.0 (Local autonomous system)".into(),
                )]
                .into_iter()
                .collect(),
                max_response_bytes: Some(32_768),
                stale_after_secs: None,
            },
        );

        let api = Arc::new(ApiHttpClient::new(Arc::new(cfg)).expect("client"));
        let tool = WikiSummaryTool { api };
        let out = tool
            .execute(json!({ "title": "Earth" }))
            .await
            .expect("execute");
        assert!(out.contains("wiki:summary"));
        assert!(out.contains("english_wikipedia"));
        assert!(out.contains("Planet"));
        assert!(out.contains("canonical_url"));
    }
}
