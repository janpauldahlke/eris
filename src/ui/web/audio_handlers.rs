//! Voice upload/preview routes — gated by [`crate::config::AudioConfig::enabled`].

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::fs;

use crate::util::audio::{normalize_upload, persist_normalized_audio, preview_filename_allowed};

use super::WebAppState;

fn audio_disabled_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "audio disabled" })),
    )
        .into_response()
}

pub async fn audio_status(State(state): State<WebAppState>) -> impl IntoResponse {
    Json(json!({ "enabled": state.config.audio.enabled }))
}

pub async fn post_audio_upload(
    State(state): State<WebAppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !state.config.audio.enabled {
        return audio_disabled_response();
    }

    let mut file_bytes: Option<Vec<u8>> = None;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name().is_some_and(|n| n == "file") {
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

    if raw.len() as u64 > state.config.audio.max_upload_bytes {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "error": format!(
                    "upload {} bytes exceeds max_upload_bytes {}",
                    raw.len(),
                    state.config.audio.max_upload_bytes
                )
            })),
        )
            .into_response();
    }

    let normalized = match normalize_upload(&raw, &state.config.audio).await {
        Ok(n) => n,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let attachment = match persist_normalized_audio(
        &state.workspace_root,
        &state.config.audio,
        normalized,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(target: "fcp.audio", error = %e, "audio upload persist failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "failed to save audio" })),
            )
                .into_response();
        }
    };

    tracing::info!(
        target: "fcp.audio",
        path = %attachment.relative_path,
        duration_secs = attachment.duration_secs,
        "audio uploaded and normalized"
    );

    (
        StatusCode::OK,
        Json(json!({
            "relative_path": attachment.relative_path,
            "preview_url": attachment.preview_url,
            "duration_secs": attachment.duration_secs,
        })),
    )
        .into_response()
}

pub async fn get_audio_preview(
    State(state): State<WebAppState>,
    Path(filename): Path<String>,
) -> impl IntoResponse {
    if !state.config.audio.enabled {
        return audio_disabled_response();
    }
    if !preview_filename_allowed(&filename) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let rel = format!(
        "{}/{}",
        state.config.audio.upload_dir.trim_end_matches('/'),
        filename
    );
    let abs = state.workspace_root.join(&rel);
    let bytes = match fs::read(&abs).await {
        Ok(b) => b,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
