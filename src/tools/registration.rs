//! Single source of truth for which optional tool families register at chat startup.
//!
//! Used by [`crate::executive::chat_session`] and the web Tools console schema.

use std::path::Path;

use crate::config::AppConfig;

pub fn should_register_moltbook(config: &AppConfig) -> bool {
    config.moltbook.enabled
}

pub fn should_register_web_search(config: &AppConfig) -> bool {
    config.web.search_enabled
}

pub fn should_register_news_today(config: &AppConfig) -> bool {
    config.news_today_enabled
}

pub fn should_register_vision(config: &AppConfig) -> bool {
    config.vision.enabled
}

pub fn should_register_weather(config: &AppConfig) -> bool {
    config.weather_enabled
}

pub fn should_register_wiki(config: &AppConfig) -> bool {
    config.wiki_enabled
}

pub fn should_register_db_rest(config: &AppConfig) -> bool {
    config.db_rest_enabled
}

pub fn should_register_google(config: &AppConfig) -> bool {
    config.google.enabled
}

/// Google mail/calendar tools register only when enabled and credentials resolve at startup.
pub fn google_credentials_complete(config: &AppConfig, workspace_root: &Path) -> bool {
    if !config.google.enabled {
        return false;
    }
    let Some(key_rel) = config.google.service_account_key.as_ref() else {
        return false;
    };
    if config.google.impersonate_user.as_deref().unwrap_or("").is_empty() {
        return false;
    }
    workspace_root.join(key_rel).is_file()
}

pub fn should_register_memory_query(config: &AppConfig, semantic_available: bool) -> bool {
    semantic_available || !config.require_semantic_brain
}

/// Open-Meteo profile ids toggled with [`AppConfig::weather_enabled`].
pub const WEATHER_API_PROFILES: &[&str] = &[
    "open_meteo_geocode",
    "open_meteo_geocode_cc",
    "open_meteo_forecast_current",
    "open_meteo_forecast_hourly",
];

pub const WIKI_API_PROFILES: &[&str] = &["wikipedia_page_summary"];

pub const DB_REST_API_PROFILES: &[&str] = &[
    "db_rest_locations",
    "db_rest_journeys_departure",
    "db_rest_journeys_arrival",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_toggles_default_on() {
        let config = AppConfig::default();
        assert!(should_register_weather(&config));
        assert!(should_register_wiki(&config));
        assert!(should_register_db_rest(&config));
    }
}
