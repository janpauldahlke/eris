//! Web console panels: identity, settings, skills, memory, uploads.

use std::path::{Component, Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs;
use uuid::Uuid;

use crate::executive::error::{FcpError, Result};
use crate::tools::vault::taglist_index::{
    SynthesisNoteCard, build_synthesis_note_cards, parse_frontmatter_string_field,
};

use super::WebAppState;
use super::settings_merge::{SettingsUpdatePayload, build_settings_schema, merge_settings_into_toml};

const SKILLS_DIR: &str = "10_Topology/skills";

fn api_error_response(e: FcpError) -> Response {
    let status = match &e {
        FcpError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({ "error": e.to_string() }))).into_response()
}

#[derive(Debug, Serialize)]
pub struct IdentityResponse {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct IdentityUpdateBody {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct SkillSummary {
    pub filename: String,
    pub id: Option<String>,
    pub title: Option<String>,
    pub priority: Option<String>,
    pub triggers: Option<String>,
    pub relative_path: String,
}

#[derive(Debug, Serialize)]
pub struct SkillDetail {
    pub summary: SkillSummary,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct MemoryNoteQuery {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub cards: Vec<SynthesisNoteCard>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UploadEntry {
    pub kind: String,
    pub filename: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub modified_unix: Option<u64>,
    pub preview_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UploadsListResponse {
    pub images: Vec<UploadEntry>,
    pub audio: Vec<UploadEntry>,
    pub files: Vec<UploadEntry>,
}

pub fn resolve_identity_path(workspace_root: &Path, config: &crate::config::AppConfig) -> PathBuf {
    let default = workspace_root.join("00_Invariants/Identity.md");
    let mut identity_path = default.clone();
    for rel in &config.vault_watch.paths {
        let norm = rel.replace('\\', "/");
        let norm_trim = norm.trim_end_matches('/');
        if norm_trim.ends_with("Identity.md") {
            identity_path = workspace_root.join(rel);
        }
    }
    identity_path
}

pub fn parse_agent_name_from_identity(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Agent Name:") {
            let name = rest
                .trim()
                .trim_end_matches("(this is you!)")
                .trim()
                .trim_end_matches('!');
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    "Agent".to_string()
}

fn vault_relative_path(workspace_root: &Path, abs: &Path) -> Result<String> {
    let rel = abs
        .strip_prefix(workspace_root)
        .map_err(|_| FcpError::Config("path outside vault".into()))?;
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(FcpError::Config("invalid vault-relative path".into())),
        }
    }
    Ok(parts.join("/"))
}

fn validate_vault_relative(path: &str, allowed_prefix: &str) -> Result<PathBuf> {
    let norm = path.replace('\\', "/");
    if norm.contains("..") || norm.starts_with('/') {
        return Err(FcpError::Config("invalid path".into()));
    }
    if !norm.starts_with(allowed_prefix) {
        return Err(FcpError::Config(format!(
            "path must start with {allowed_prefix}"
        )));
    }
    Ok(PathBuf::from(norm))
}

pub async fn get_identity(State(state): State<WebAppState>) -> impl IntoResponse {
    let path = resolve_identity_path(&state.workspace_root, &state.config);
    let content = match fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) => return api_error_response(FcpError::Io(e)),
    };
    let rel = match vault_relative_path(&state.workspace_root, &path) {
        Ok(r) => r,
        Err(e) => return api_error_response(e),
    };
    Json(IdentityResponse { path: rel, content }).into_response()
}

pub async fn put_identity(
    State(state): State<WebAppState>,
    Json(body): Json<IdentityUpdateBody>,
) -> impl IntoResponse {
    if body.content.trim().is_empty() {
        return api_error_response(FcpError::Config("identity content must not be empty".into()));
    }
    let path = resolve_identity_path(&state.workspace_root, &state.config);
    if let Err(e) = fs::write(&path, body.content.as_bytes()).await {
        return api_error_response(FcpError::Io(e));
    }
    tracing::info!(
        event = "fcp.web.console.identity_saved",
        path = %path.display(),
        "Identity.md updated from web console"
    );
    Json(json!({ "ok": true })).into_response()
}

pub async fn get_settings(State(state): State<WebAppState>) -> Json<super::settings_merge::SettingsSchemaResponse> {
    Json(build_settings_schema(&state.config))
}

pub async fn put_settings(
    State(state): State<WebAppState>,
    Json(payload): Json<SettingsUpdatePayload>,
) -> impl IntoResponse {
    if let Err(e) =
        merge_settings_into_toml(&state.workspace_root, &state.config, &payload).await
    {
        return api_error_response(e);
    }
    Json(json!({ "ok": true, "restart_required": true })).into_response()
}

pub async fn get_skills(State(state): State<WebAppState>) -> impl IntoResponse {
    let dir = state.workspace_root.join(SKILLS_DIR);
    let mut out: Vec<SkillSummary> = Vec::new();
    let mut read = match fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Json(out).into_response(),
        Err(e) => return api_error_response(FcpError::Io(e)),
    };
    while let Some(entry) = match read.next_entry().await {
        Ok(v) => v,
        Err(e) => return api_error_response(FcpError::Io(e)),
    } {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        let raw = match fs::read_to_string(&path).await {
            Ok(r) => r,
            Err(e) => return api_error_response(FcpError::Io(e)),
        };
        let relative_path = format!("{SKILLS_DIR}/{filename}");
        out.push(SkillSummary {
            filename: filename.clone(),
            id: parse_frontmatter_string_field(&raw, "id"),
            title: parse_frontmatter_string_field(&raw, "title"),
            priority: parse_frontmatter_string_field(&raw, "priority"),
            triggers: parse_frontmatter_string_field(&raw, "triggers"),
            relative_path,
        });
    }
    out.sort_by(|a, b| {
        a.title
            .as_deref()
            .unwrap_or(&a.filename)
            .to_lowercase()
            .cmp(&b.title.as_deref().unwrap_or(&b.filename).to_lowercase())
    });
    Json(out).into_response()
}

pub async fn get_skill_detail(
    State(state): State<WebAppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> impl IntoResponse {
    if name.contains('/') || name.contains("..") || !name.ends_with(".md") {
        return api_error_response(FcpError::Config("invalid skill filename".into()));
    }
    let path = state.workspace_root.join(SKILLS_DIR).join(&name);
    let raw = match fs::read_to_string(&path).await {
        Ok(r) => r,
        Err(e) => return api_error_response(FcpError::Io(e)),
    };
    let body = strip_frontmatter(&raw);
    let summary = SkillSummary {
        filename: name.clone(),
        id: parse_frontmatter_string_field(&raw, "id"),
        title: parse_frontmatter_string_field(&raw, "title"),
        priority: parse_frontmatter_string_field(&raw, "priority"),
        triggers: parse_frontmatter_string_field(&raw, "triggers"),
        relative_path: format!("{SKILLS_DIR}/{name}"),
    };
    Json(SkillDetail { summary, body }).into_response()
}

fn strip_frontmatter(raw: &str) -> String {
    let rest = match raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n")) {
        Some(r) => r,
        None => return raw.to_string(),
    };
    let Some(end) = rest.find("\n---") else {
        return raw.to_string();
    };
    let after = &rest[end + 4..];
    after.trim_start_matches('\n').trim_start_matches('\r').to_string()
}

pub async fn get_memory(State(state): State<WebAppState>) -> impl IntoResponse {
    let cards = match build_synthesis_note_cards(&state.workspace_root).await {
        Ok(c) => c,
        Err(e) => return api_error_response(e),
    };
    let mut tags: Vec<String> = Vec::new();
    for card in &cards {
        for t in &card.tags {
            if !tags.contains(t) {
                tags.push(t.clone());
            }
        }
    }
    tags.sort();
    Json(MemoryListResponse { cards, tags }).into_response()
}

pub async fn get_memory_note(
    State(state): State<WebAppState>,
    Query(q): Query<MemoryNoteQuery>,
) -> impl IntoResponse {
    let rel = match validate_vault_relative(&q.path, "30_Synthesis/") {
        Ok(r) => r,
        Err(e) => return api_error_response(e),
    };
    let abs = state.workspace_root.join(&rel);
    let raw = match fs::read_to_string(&abs).await {
        Ok(r) => r,
        Err(e) => return api_error_response(FcpError::Io(e)),
    };
    let title = parse_frontmatter_string_field(&raw, "title");
    let tags = crate::tools::vault::taglist_index::parse_frontmatter_tags(&raw);
    let epistemic_status = parse_frontmatter_string_field(&raw, "epistemic_status");
    let body = strip_frontmatter(&raw);
    Json(json!({
        "path": rel.to_string_lossy().to_string(),
        "title": title,
        "tags": tags,
        "epistemic_status": epistemic_status,
        "body": body,
    }))
    .into_response()
}

pub async fn get_uploads(State(state): State<WebAppState>) -> impl IntoResponse {
    let cfg = &state.config;
    let images_dir = state.workspace_root.join(&cfg.vision.upload_dir);
    let audio_dir = state.workspace_root.join(&cfg.audio.upload_dir);
    let files_dir = state
        .workspace_root
        .join(&cfg.web_ui.uploads.files.upload_dir);

    let images = match list_upload_dir(
        &state.workspace_root,
        &images_dir,
        "image",
        |f| Some(format!("/api/vision/preview/{f}")),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return api_error_response(e),
    };
    let audio = match list_upload_dir(
        &state.workspace_root,
        &audio_dir,
        "audio",
        |f| Some(format!("/api/audio/preview/{f}")),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return api_error_response(e),
    };
    let files = match list_upload_dir(&state.workspace_root, &files_dir, "file", |_| None).await {
        Ok(v) => v,
        Err(e) => return api_error_response(e),
    };

    Json(UploadsListResponse {
        images,
        audio,
        files,
    })
    .into_response()
}

async fn list_upload_dir<F>(
    workspace_root: &Path,
    dir: &Path,
    kind: &str,
    preview_url: F,
) -> Result<Vec<UploadEntry>>
where
    F: Fn(&str) -> Option<String>,
{
    let mut out = Vec::new();
    let mut read = match fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(FcpError::Io(e)),
    };
    while let Some(entry) = read.next_entry().await.map_err(FcpError::Io)? {
        let ft = entry.file_type().await.map_err(FcpError::Io)?;
        if !ft.is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        if filename.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let meta = fs::metadata(&path).await.map_err(FcpError::Io)?;
        let modified_unix = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let rel = vault_relative_path(workspace_root, &path).unwrap_or_else(|_| filename.clone());
        out.push(UploadEntry {
            kind: kind.to_string(),
            filename: filename.clone(),
            relative_path: rel,
            size_bytes: meta.len(),
            modified_unix,
            preview_url: preview_url(&filename),
        });
    }
    out.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix));
    Ok(out)
}

pub async fn post_upload_file(
    State(state): State<WebAppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let cfg = &state.config.web_ui.uploads.files;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut orig_name: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name().is_some_and(|n| n == "file") {
            orig_name = field.file_name().map(str::to_string);
            match field.bytes().await {
                Ok(b) => file_bytes = Some(b.to_vec()),
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": format!("read upload: {e}") })),
                    )
                        .into_response();
                }
            }
            break;
        }
    }

    let raw = match file_bytes {
        Some(b) => b,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "missing multipart field 'file'" })),
            )
                .into_response();
        }
    };

    if raw.len() as u64 > cfg.max_upload_bytes {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "error": format!(
                    "upload {} bytes exceeds max_upload_bytes {}",
                    raw.len(),
                    cfg.max_upload_bytes
                )
            })),
        )
            .into_response();
    }

    let ext = orig_name
        .as_deref()
        .and_then(|n| n.rsplit('.').next())
        .map(str::to_lowercase)
        .unwrap_or_default();
    let allowed: Vec<String> = cfg
        .allowed_extensions
        .iter()
        .map(|e| e.to_lowercase())
        .collect();
    if !allowed.contains(&ext) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": format!(
                    "extension '{ext}' not allowed; permitted: {}",
                    allowed.join(", ")
                )
            })),
        )
            .into_response();
    }

    let upload_dir = state.workspace_root.join(&cfg.upload_dir);
    if let Err(e) = fs::create_dir_all(&upload_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("mkdir: {e}") })),
        )
            .into_response();
    }

    let stored_name = format!("{}.{}", Uuid::new_v4(), ext);
    let dest = upload_dir.join(&stored_name);
    if let Err(e) = fs::write(&dest, &raw).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("write: {e}") })),
        )
            .into_response();
    }

    let relative_path = format!("{}/{}", cfg.upload_dir.trim_end_matches('/'), stored_name);
    tracing::info!(
        event = "fcp.web.console.file_uploaded",
        path = %relative_path,
        bytes = raw.len(),
        "User file stored in vault upload dir"
    );

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "relative_path": relative_path,
            "filename": stored_name,
            "size_bytes": raw.len(),
        })),
    )
        .into_response()
}

pub async fn console_js() -> Response {
    match Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )
        .body(Body::from(include_str!("assets/console.js")))
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to build console.js response");
            Response::new(Body::from("/* asset error */"))
        }
    }
}
