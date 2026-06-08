use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};

use super::validate::validate_audio_relative_path;

#[derive(Serialize)]
struct ChatMsgContentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_audio: Option<InputAudioPart>,
}

#[derive(Serialize)]
struct InputAudioPart {
    data: String,
    format: String,
}

#[derive(Serialize)]
struct ChatMsg {
    role: String,
    content: Vec<ChatMsgContentPart>,
}

#[derive(Serialize)]
struct TranscribeRequest {
    messages: Vec<ChatMsg>,
    stream: bool,
    temperature: f32,
    chat_template_kwargs: ChatTemplateKwargs,
}

#[derive(Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

#[derive(Deserialize)]
struct TranscribeResponse {
    choices: Vec<TranscribeChoice>,
}

#[derive(Deserialize)]
struct TranscribeChoice {
    message: Option<TranscribeMessage>,
}

#[derive(Deserialize)]
struct TranscribeMessage {
    content: Option<String>,
}

/// Verbatim STT via llama-server `input_audio` (no GBNF grammar).
pub async fn transcribe_audio(
    config: &Arc<AppConfig>,
    workspace_root: &Path,
    relative_path: &str,
) -> Result<String> {
    let abs = validate_audio_relative_path(
        workspace_root,
        &config.audio.upload_dir,
        relative_path,
    )?;
    let bytes = fs::read(&abs).await.map_err(FcpError::Io)?;
    let b64 = BASE64.encode(&bytes);
    let prompt = config.audio.transcription_prompt.clone();

    let lc = config.validate_llamacpp_config()?;
    let chat_url = format!(
        "{}/v1/chat/completions",
        lc.chat_server_url.trim_end_matches('/')
    );
    let body = TranscribeRequest {
        messages: vec![ChatMsg {
            role: "user".into(),
            content: vec![
                ChatMsgContentPart {
                    kind: "text".into(),
                    text: Some(prompt),
                    input_audio: None,
                },
                ChatMsgContentPart {
                    kind: "input_audio".into(),
                    text: None,
                    input_audio: Some(InputAudioPart {
                        data: b64,
                        format: "wav".into(),
                    }),
                },
            ],
        }],
        stream: false,
        temperature: 0.2,
        chat_template_kwargs: ChatTemplateKwargs {
            enable_thinking: false,
        },
    };

    let timeout = Duration::from_secs(config.generation_timeout_secs);
    let http = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| FcpError::NetworkFault(format!("audio STT HTTP client: {e}")))?;

    let response = http
        .post(&chat_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("audio STT request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let excerpt = response
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(500)
            .collect::<String>();
        return Err(FcpError::NetworkFault(format!(
            "audio STT llama-server HTTP {status}: {excerpt}"
        )));
    }

    let parsed: TranscribeResponse = response
        .json()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("audio STT response parse: {e}")))?;

    let content = parsed
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    Ok(content.trim().to_string())
}
