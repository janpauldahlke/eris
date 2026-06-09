//! Tool family catalog and operator-facing schema for the web Tools console.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::config::AppConfig;
use crate::tools::registration::{
    google_credentials_complete, should_register_db_rest, should_register_google,
    should_register_moltbook, should_register_news_today, should_register_vision,
    should_register_weather, should_register_wiki,
};
use crate::tools::ToolDescriptorRegistry;

use super::settings_merge::SettingsFieldSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolFamilyStatus {
    Core,
    Active,
    Off,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolFamilyResponse {
    pub id: String,
    pub label: String,
    pub summary: String,
    pub tool_names: Vec<String>,
    pub status: ToolFamilyStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_hint: Option<String>,
    pub fields: Vec<SettingsFieldSchema>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolsSchemaResponse {
    pub families: Vec<ToolFamilyResponse>,
}

struct ToolFamilyDef {
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    tool_names: &'static [&'static str],
    is_core: bool,
}

const FAMILIES: &[ToolFamilyDef] = &[
    ToolFamilyDef {
        id: "vault",
        label: "Vault",
        summary: "Read, write, list, search, and tag-index markdown under the vault.",
        tool_names: &[
            "vault:read",
            "vault:write",
            "vault:list",
            "vault:search",
            "vault:taglist",
        ],
        is_core: true,
    },
    ToolFamilyDef {
        id: "skills",
        label: "Skills",
        summary: "List, read, and create topology skills under 10_Topology/skills/.",
        tool_names: &["skills:list", "skills:read", "skills:create"],
        is_core: true,
    },
    ToolFamilyDef {
        id: "memory",
        label: "Memory",
        summary: "Stage ephemeral notes, commit to Qdrant, and query semantic memory.",
        tool_names: &[
            "memory:stage",
            "memory:staged_list",
            "memory:commit",
            "memory:commit_all",
            "memory:query",
        ],
        is_core: true,
    },
    ToolFamilyDef {
        id: "agenda",
        label: "Agenda",
        summary: "Queue background tasks, reminders, and alarms.",
        tool_names: &[
            "agenda:push",
            "agenda:list",
            "agenda:remind_at",
            "agenda:remind_self",
            "agenda:complete",
            "agenda:remove",
        ],
        is_core: true,
    },
    ToolFamilyDef {
        id: "clock",
        label: "Clock",
        summary: "Current time, timers, and wall-clock alarms.",
        tool_names: &["clock:now", "clock:timer", "clock:alarm"],
        is_core: true,
    },
    ToolFamilyDef {
        id: "system",
        label: "System",
        summary: "Runtime health and configuration snapshot for the operator.",
        tool_names: &["system:health"],
        is_core: true,
    },
    ToolFamilyDef {
        id: "web",
        label: "Web",
        summary: "Fetch allowlisted pages, search via browser39, and query cached artifacts.",
        tool_names: &["web:fetch", "web:find", "web:search"],
        is_core: false,
    },
    ToolFamilyDef {
        id: "news",
        label: "News",
        summary: "Homepage headline digest (BBC by default) with optional deep article fetch.",
        tool_names: &["news:today"],
        is_core: false,
    },
    ToolFamilyDef {
        id: "weather",
        label: "Weather",
        summary: "Open-Meteo current conditions and short forecast via geocoding.",
        tool_names: &["weather:current", "weather:forecast"],
        is_core: false,
    },
    ToolFamilyDef {
        id: "wiki",
        label: "Wikipedia",
        summary: "English Wikipedia REST page summaries.",
        tool_names: &["wiki:summary"],
        is_core: false,
    },
    ToolFamilyDef {
        id: "trains",
        label: "Trains (DB)",
        summary: "Deutsche Bahn-style journey search via v6.db.transport.rest.",
        tool_names: &["db:find_connections"],
        is_core: false,
    },
    ToolFamilyDef {
        id: "google",
        label: "Google Workspace",
        summary: "Gmail and Google Calendar via service-account domain-wide delegation.",
        tool_names: &[
            "mail:check",
            "mail:read",
            "mail:digest",
            "mail:delete",
            "mail:move",
            "mail:write",
            "calendar:list",
            "calendar:get",
            "calendar:create",
            "calendar:update",
            "calendar:delete",
        ],
        is_core: false,
    },
    ToolFamilyDef {
        id: "moltbook",
        label: "Moltbook",
        summary: "Native tools for the Moltbook agent social network.",
        tool_names: &[
            "moltbook:register",
            "moltbook:status",
            "moltbook:home",
            "moltbook:feed",
            "moltbook:search",
            "moltbook:comments",
            "moltbook:comment",
            "moltbook:vote",
            "moltbook:post",
            "moltbook:verify",
            "moltbook:notifications_read",
            "moltbook:dm",
        ],
        is_core: false,
    },
    ToolFamilyDef {
        id: "vision",
        label: "Vision",
        summary: "Multimodal image describe (vision:see) and inline web display (vision:display).",
        tool_names: &["vision:see", "vision:display"],
        is_core: false,
    },
    ToolFamilyDef {
        id: "media",
        label: "Media catalog",
        summary: "Create and update 40_MEDIA catalog cards for remembered images.",
        tool_names: &["media:catalog", "media:meta"],
        is_core: true,
    },
    ToolFamilyDef {
        id: "audio",
        label: "Audio",
        summary: "Voice upload and mic transcription in the web compose area (not gatekeeper tools).",
        tool_names: &[],
        is_core: false,
    },
    ToolFamilyDef {
        id: "discord",
        label: "Discord",
        summary: "Optional Serenity sidecar sharing this chat session (not gatekeeper tools).",
        tool_names: &[],
        is_core: false,
    },
];

pub fn build_tools_schema(
    config: &AppConfig,
    workspace_root: &Path,
    registered_tool_names: &[String],
) -> ToolsSchemaResponse {
    let registered: HashSet<&str> = registered_tool_names.iter().map(String::as_str).collect();
    let descriptor_registry = ToolDescriptorRegistry::load_embedded().ok();

    let families = FAMILIES
        .iter()
        .map(|def| {
            let tool_names: Vec<String> = def.tool_names.iter().map(|s| (*s).to_string()).collect();
            let (status, status_reason) = compute_status(
                def,
                config,
                workspace_root,
                &registered,
                &tool_names,
            );
            let agent_hint = def
                .tool_names
                .first()
                .and_then(|name| {
                    descriptor_registry
                        .as_ref()
                        .and_then(|r| r.get(name))
                        .map(|d| d.short_description.clone())
                });
            let fields = fields_for_family(def.id, config, workspace_root);
            ToolFamilyResponse {
                id: def.id.to_string(),
                label: def.label.to_string(),
                summary: def.summary.to_string(),
                tool_names,
                status,
                status_reason,
                agent_hint,
                fields,
            }
        })
        .collect();

    ToolsSchemaResponse { families }
}

fn compute_status(
    def: &ToolFamilyDef,
    config: &AppConfig,
    workspace_root: &Path,
    registered: &HashSet<&str>,
    tool_names: &[String],
) -> (ToolFamilyStatus, Option<String>) {
    if def.is_core {
        if def.id == "memory" {
            let query_registered = registered.contains("memory:query");
            if config.require_semantic_brain && !query_registered {
                return (
                    ToolFamilyStatus::Unavailable,
                    Some("Qdrant semantic brain required but memory:query not registered.".into()),
                );
            }
        }
        return (ToolFamilyStatus::Core, None);
    }

    let config_enabled = family_config_enabled(def.id, config);
    if !config_enabled {
        return (ToolFamilyStatus::Off, None);
    }

    if def.id == "google" {
        if !google_credentials_complete(config, workspace_root) {
            return (
                ToolFamilyStatus::Unavailable,
                Some(
                    "google.enabled is true but service account key or impersonate_user is missing."
                        .into(),
                ),
            );
        }
    }

    if def.id == "moltbook" && should_register_moltbook(config) {
        let any_registered = tool_names.iter().any(|n| registered.contains(n.as_str()));
        if !any_registered {
            return (
                ToolFamilyStatus::Unavailable,
                Some(
                    "Moltbook enabled but API credentials missing (api_key_file or env)."
                        .into(),
                ),
            );
        }
    }

    if def.id == "vision" && should_register_vision(config) {
        if let Err(e) = config.validate_vision_ready() {
            return (ToolFamilyStatus::Unavailable, Some(e.to_string()));
        }
    }

    if def.id == "audio" {
        if config.audio.enabled {
            if let Err(e) = config.validate_audio_ready() {
                return (ToolFamilyStatus::Unavailable, Some(e.to_string()));
            }
            return (ToolFamilyStatus::Active, None);
        }
        return (ToolFamilyStatus::Off, None);
    }

    if def.id == "discord" {
        if config.discord.enabled {
            if config.discord_sidecar_should_run() {
                return (ToolFamilyStatus::Active, None);
            }
            return (
                ToolFamilyStatus::Unavailable,
                Some(
                    "discord.enabled but bot_token or channel target not configured.".into(),
                ),
            );
        }
        return (ToolFamilyStatus::Off, None);
    }

    if tool_names.is_empty() {
        return (ToolFamilyStatus::Off, None);
    }

    let any_active = tool_names.iter().any(|n| registered.contains(n.as_str()));
    if any_active {
        (ToolFamilyStatus::Active, None)
    } else {
        (
            ToolFamilyStatus::Unavailable,
            Some("Enabled in config but tools not registered at startup.".into()),
        )
    }
}

fn family_config_enabled(family_id: &str, config: &AppConfig) -> bool {
    match family_id {
        "web" => true,
        "news" => should_register_news_today(config),
        "weather" => should_register_weather(config),
        "wiki" => should_register_wiki(config),
        "trains" => should_register_db_rest(config),
        "google" => should_register_google(config),
        "moltbook" => should_register_moltbook(config),
        "vision" => should_register_vision(config),
        "audio" => config.audio.enabled,
        "discord" => config.discord.enabled,
        _ => true,
    }
}

fn fields_for_family(
    family_id: &str,
    config: &AppConfig,
    workspace_root: &Path,
) -> Vec<SettingsFieldSchema> {
    match family_id {
        "vault" => vec![
            field_f64(
                "vault_read_ratio",
                f64::from(config.vault_read_ratio),
                "Vault read ratio",
                "Fraction of num_ctx used as the vault:read character budget.",
                "Higher values allow longer file reads per tool call.",
                true,
            ),
            field_usize(
                "vault_search_max_files",
                config.vault_search_max_files as usize,
                "Search max files",
                "Maximum files returned by vault:search.",
                "More files add context but consume tokens.",
                true,
                None,
                None,
            ),
            field_usize(
                "vault_search_max_total_chars",
                config.vault_search_max_total_chars as usize,
                "Search max total chars",
                "Total character cap across vault:search snippets.",
                "Bounds search output size.",
                true,
                None,
                None,
            ),
        ],
        "memory" => vec![
            field_usize(
                "memory_query_default_top_k",
                config.memory_query_default_top_k as usize,
                "Query default top K",
                "Default number of Qdrant hits for memory:query.",
                "More hits add context.",
                true,
                None,
                None,
            ),
            field_usize(
                "memory_query_top_k_max",
                config.memory_query_top_k_max as usize,
                "Query top K max",
                "Hard cap the model may request for memory:query.",
                "Prevents oversized vector queries.",
                true,
                None,
                None,
            ),
            field_usize(
                "memory_query_default_max_total_chars",
                config.memory_query_default_max_total_chars as usize,
                "Query max total chars",
                "Default character budget for memory:query results.",
                "Bounds injected memory text.",
                true,
                None,
                None,
            ),
            field_bool(
                "require_semantic_brain",
                config.require_semantic_brain,
                "Require semantic brain",
                "When true, chat startup fails if Qdrant is unreachable.",
                "Disabling allows chat without memory:query/commit.",
                true,
            ),
        ],
        "web" => vec![
            field_bool(
                "web.search_enabled",
                config.web.search_enabled,
                "Web search enabled",
                "Register web:search (browser39 search engine).",
                "When false, only web:fetch and web:find are available.",
                true,
            ),
            field_u64(
                "web.default_fetch_budget",
                u64::from(config.web.default_fetch_budget),
                "Default fetch budget",
                "Starting fetch budget per chat session for web tools.",
                "Higher values allow more page fetches.",
                true,
            ),
            field_u64(
                "web.max_fetches_per_user_turn",
                u64::from(config.web.max_fetches_per_user_turn),
                "Max fetches per turn",
                "Cap on web:fetch calls per user turn.",
                "Limits crawl depth per message.",
                true,
            ),
            field_u64(
                "web.max_web_tool_calls_per_turn",
                u64::from(config.web.max_web_tool_calls_per_turn),
                "Max web tool calls per turn",
                "Combined cap on web:fetch, web:find, and web:search per turn.",
                "Prevents runaway browsing.",
                true,
            ),
            field_bool(
                "web.require_find_before_refetch",
                config.web.require_find_before_refetch,
                "Require find before refetch",
                "Force web:find on a cached artifact before repeating web:fetch on the same URL.",
                "Reduces redundant fetches.",
                true,
            ),
            field_bool(
                "web.allowlist_enabled",
                config.web.allowlist_enabled,
                "Allowlist enabled",
                "Enforce .fcp/web_allowlist.toml host patterns.",
                "Disable only for local dev.",
                true,
            ),
            field_bool(
                "web.explore_site_enabled",
                config.web.explore_site_enabled,
                "Explore site enabled",
                "Allow multi-page explore missions under web budgets.",
                "Enables deeper site crawling when the agent plans it.",
                true,
            ),
        ],
        "news" => vec![
            field_bool(
                "news_today_enabled",
                config.news_today_enabled,
                "News today enabled",
                "Register the news:today tool.",
                "When false, headline digest is unavailable.",
                true,
            ),
            field_string(
                "news_today_site_base",
                config.news_today_site_base.clone(),
                "News site base",
                "Origin for category-relative news:today URLs (default BBC).",
                "Used when the agent passes a category, not a full homepage URL.",
                true,
            ),
            field_string_opt(
                "news_today_default_homepage",
                config.news_today_default_homepage.clone(),
                "Default homepage",
                "Listing URL when news:today omits homepage_url and category.",
                "Default BBC home unless overridden.",
                true,
            ),
            field_usize(
                "news_today_max_headlines_default",
                config.news_today_max_headlines_default,
                "Max headlines default",
                "Default headline count for news:today.",
                "Higher values produce longer digests.",
                true,
                None,
                None,
            ),
            field_u8(
                "news_today_deep_fetch_max_default",
                config.news_today_deep_fetch_max_default,
                "Deep fetch max default",
                "Default number of full article bodies to fetch (0–3).",
                "Non-zero adds latency and token use.",
                true,
            ),
        ],
        "weather" => vec![field_bool(
            "weather_enabled",
            config.weather_enabled,
            "Weather enabled",
            "Register weather:current and weather:forecast.",
            "Uses Open-Meteo API profiles in [apis.*].",
            true,
        )],
        "wiki" => vec![field_bool(
            "wiki_enabled",
            config.wiki_enabled,
            "Wikipedia enabled",
            "Register wiki:summary.",
            "Uses the wikipedia_page_summary API profile.",
            true,
        )],
        "trains" => vec![field_bool(
            "db_rest_enabled",
            config.db_rest_enabled,
            "DB REST enabled",
            "Register db:find_connections for train journey search.",
            "Uses v6.db.transport.rest API profiles.",
            true,
        )],
        "google" => {
            let key_display = config
                .google
                .service_account_key
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let key_exists = config
                .google
                .service_account_key
                .as_ref()
                .is_some_and(|p| workspace_root.join(p).is_file());
            vec![
                field_bool(
                    "google.enabled",
                    config.google.enabled,
                    "Google enabled",
                    "Register Gmail and Calendar tools when credentials resolve.",
                    "Requires service account + domain-wide delegation.",
                    true,
                ),
                field_string(
                    "google.service_account_key",
                    key_display,
                    "Service account key path",
                    "Path to the JSON key under the vault (e.g. .fcp/eris-sa.json).",
                    if key_exists {
                        "Key file found on disk."
                    } else {
                        "Key file not found — tools will not register until present."
                    },
                    true,
                ),
                field_string(
                    "google.impersonate_user",
                    config
                        .google
                        .impersonate_user
                        .clone()
                        .unwrap_or_default(),
                    "Impersonate user",
                    "Workspace user email for domain-wide delegation.",
                    "Must match a user authorized in Google Admin Console.",
                    true,
                ),
            ]
        }
        "moltbook" => vec![
            field_bool(
                "moltbook.enabled",
                config.moltbook.enabled,
                "Moltbook enabled",
                "Register native moltbook:* tools at startup.",
                "Requires API credentials file or env var.",
                true,
            ),
            field_string_opt_path(
                "moltbook.api_key_file",
                config.moltbook.api_key_file.as_ref(),
                "API key file",
                "JSON credentials path (e.g. .fcp/moltbook/credentials.json).",
                "Used when MOLTBOOK_API_KEY env is unset.",
                true,
            ),
            field_string_opt(
                "moltbook.agent_name",
                config.moltbook.agent_name.clone(),
                "Agent name",
                "Expected agent name for operator-facing status messages.",
                "Informational; does not change API auth.",
                true,
            ),
            field_u64(
                "moltbook.timeout_secs",
                config.moltbook.timeout_secs,
                "HTTP timeout (seconds)",
                "Wall clock for each Moltbook API request.",
                "Raise if comments/DM payloads time out.",
                true,
            ),
            field_usize(
                "moltbook.max_response_bytes",
                config.moltbook.max_response_bytes,
                "Max response bytes",
                "UTF-8 byte cap per Moltbook JSON response before parse.",
                "Too low truncates large comment threads.",
                true,
                None,
                None,
            ),
        ],
        "vision" => vec![field_bool(
            "vision.enabled",
            config.vision.enabled,
            "Vision enabled",
            "Register vision:see and vision:display; enable web image upload.",
            "Requires LlamaCpp backend and llama_cpp.mmproj_path.",
            true,
        )],
        "audio" => vec![
            field_bool(
                "audio.enabled",
                config.audio.enabled,
                "Audio enabled",
                "Enable voice upload and mic in the web compose area.",
                "Requires LlamaCpp backend and mmproj for transcription.",
                true,
            ),
            field_u64(
                "audio.max_duration_secs",
                u64::from(config.audio.max_duration_secs),
                "Max recording duration",
                "Upper bound on voice clip length (seconds).",
                "Longer clips use more upload and processing time.",
                true,
            ),
        ],
        "discord" => vec![
            field_bool(
                "discord.enabled",
                config.discord.enabled,
                "Discord enabled",
                "Start the Serenity Discord sidecar with this session.",
                "Requires bot_token and channel_id or channel_name.",
                true,
            ),
            field_string_opt(
                "discord.channel_name",
                config.discord.channel_name.clone(),
                "Channel name",
                "Exact guild text channel name resolved at READY.",
                "Alternative to numeric channel_id.",
                true,
            ),
            field_string_opt_u64(
                "discord.channel_id",
                config.discord.channel_id,
                "Channel ID",
                "Numeric snowflake for the target text channel.",
                "Takes precedence over channel_name when set.",
                true,
            ),
            field_string_readonly(
                "discord.bot_token",
                if config.discord.bot_token.as_ref().is_some_and(|t| !t.is_empty()) {
                    "(configured — edit in config.toml)".into()
                } else {
                    "(not set)".into()
                },
                "Bot token",
                "Discord bot token from Developer Portal.",
                "Not editable via web UI for security.",
            ),
        ],
        _ => Vec::new(),
    }
}

fn field_bool(key: &str, value: bool, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_u64(key: &str, value: u64, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_u8(key: &str, value: u8, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_usize(
    key: &str,
    value: usize,
    label: &str,
    description: &str,
    impact: &str,
    editable: bool,
    max_value: Option<usize>,
    warn_above: Option<usize>,
) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::from(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value,
        warn_above,
    }
}

fn field_f64(key: &str, value: f64, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: serde_json::Number::from_f64(value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::from(0)),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_string(key: &str, value: String, label: &str, description: &str, impact: &str, editable: bool) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::String(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: true,
        editable,
        max_value: None,
        warn_above: None,
    }
}

fn field_string_opt(
    key: &str,
    value: Option<String>,
    label: &str,
    description: &str,
    impact: &str,
    editable: bool,
) -> SettingsFieldSchema {
    field_string(
        key,
        value.unwrap_or_default(),
        label,
        description,
        impact,
        editable,
    )
}

fn field_string_opt_path(
    key: &str,
    value: Option<&std::path::PathBuf>,
    label: &str,
    description: &str,
    impact: &str,
    editable: bool,
) -> SettingsFieldSchema {
    field_string(
        key,
        value.map(|p| p.display().to_string()).unwrap_or_default(),
        label,
        description,
        impact,
        editable,
    )
}

fn field_string_opt_u64(
    key: &str,
    value: Option<u64>,
    label: &str,
    description: &str,
    impact: &str,
    editable: bool,
) -> SettingsFieldSchema {
    field_string(
        key,
        value.map(|v| v.to_string()).unwrap_or_default(),
        label,
        description,
        impact,
        editable,
    )
}

fn field_string_readonly(key: &str, value: String, label: &str, description: &str, impact: &str) -> SettingsFieldSchema {
    SettingsFieldSchema {
        key: key.to_string(),
        value: JsonValue::String(value),
        label: label.to_string(),
        description: description.to_string(),
        impact: impact.to_string(),
        restart_required: false,
        editable: false,
        max_value: None,
        warn_above: None,
    }
}

pub fn family_field_keys(family_id: &str) -> Vec<&'static str> {
    match family_id {
        "vault" => vec![
            "vault_read_ratio",
            "vault_search_max_files",
            "vault_search_max_total_chars",
        ],
        "memory" => vec![
            "memory_query_default_top_k",
            "memory_query_top_k_max",
            "memory_query_default_max_total_chars",
            "require_semantic_brain",
        ],
        "web" => vec![
            "web.search_enabled",
            "web.default_fetch_budget",
            "web.max_fetches_per_user_turn",
            "web.max_web_tool_calls_per_turn",
            "web.require_find_before_refetch",
            "web.allowlist_enabled",
            "web.explore_site_enabled",
        ],
        "news" => vec![
            "news_today_enabled",
            "news_today_site_base",
            "news_today_default_homepage",
            "news_today_max_headlines_default",
            "news_today_deep_fetch_max_default",
        ],
        "weather" => vec!["weather_enabled"],
        "wiki" => vec!["wiki_enabled"],
        "trains" => vec!["db_rest_enabled"],
        "google" => vec![
            "google.enabled",
            "google.service_account_key",
            "google.impersonate_user",
        ],
        "moltbook" => vec![
            "moltbook.enabled",
            "moltbook.api_key_file",
            "moltbook.agent_name",
            "moltbook.timeout_secs",
            "moltbook.max_response_bytes",
        ],
        "vision" => vec!["vision.enabled"],
        "audio" => vec!["audio.enabled", "audio.max_duration_secs"],
        "discord" => vec![
            "discord.enabled",
            "discord.channel_name",
            "discord.channel_id",
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_tool_names_have_descriptors_when_non_empty() {
        let registry = ToolDescriptorRegistry::load_embedded().expect("descriptors");
        assert!(!registry.is_empty());
        for def in FAMILIES {
            for name in def.tool_names {
                assert!(
                    registry.get(name).is_some(),
                    "missing descriptor for {} in family {}",
                    name,
                    def.id
                );
            }
        }
    }
}
