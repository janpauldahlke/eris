use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::executive::error::{FcpError, Result};

#[derive(Serialize)]
struct ChatMsgContentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<ImageUrlPart>,
}

#[derive(Serialize)]
struct ImageUrlPart {
    url: String,
}

#[derive(Serialize)]
struct ChatMsg {
    role: String,
    content: Vec<ChatMsgContentPart>,
}

#[derive(Serialize)]
struct VisionRequest {
    messages: Vec<ChatMsg>,
    stream: bool,
    temperature: f32,
}

#[derive(Deserialize)]
struct VisionResponse {
    choices: Vec<VisionChoice>,
    usage: Option<VisionUsage>,
}

#[derive(Deserialize)]
struct VisionChoice {
    message: Option<VisionMessage>,
}

#[derive(Deserialize)]
struct VisionMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct VisionUsage {
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
}

/// Multimodal describe call — no GBNF grammar.
pub async fn vision_describe(
    config: &Arc<AppConfig>,
    relative_path: &str,
    prompt: &str,
) -> Result<(String, usize, usize)> {
    let lc = config.validate_llamacpp_config()?;
    let chat_url = format!(
        "{}/v1/chat/completions",
        lc.chat_server_url.trim_end_matches('/')
    );
    let file_url = format!("file://{}", relative_path.replace('\\', "/"));
    let body = VisionRequest {
        messages: vec![ChatMsg {
            role: "user".into(),
            content: vec![
                ChatMsgContentPart {
                    kind: "text".into(),
                    text: Some(prompt.to_string()),
                    image_url: None,
                },
                ChatMsgContentPart {
                    kind: "image_url".into(),
                    text: None,
                    image_url: Some(ImageUrlPart { url: file_url }),
                },
            ],
        }],
        stream: false,
        temperature: 0.2,
    };

    let timeout = Duration::from_secs(config.generation_timeout_secs);
    let http = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| FcpError::NetworkFault(format!("vision HTTP client: {e}")))?;

    let response = http
        .post(&chat_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("vision request failed: {e}")))?;

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
            "vision llama-server HTTP {status}: {excerpt}"
        )));
    }

    let parsed: VisionResponse = response
        .json()
        .await
        .map_err(|e| FcpError::NetworkFault(format!("vision response parse: {e}")))?;

    let content = parsed
        .choices
        .first()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    let pt = parsed
        .usage
        .as_ref()
        .and_then(|u| u.prompt_tokens)
        .unwrap_or(0);
    let ct = parsed
        .usage
        .as_ref()
        .and_then(|u| u.completion_tokens)
        .unwrap_or(0);
    Ok((content, pt, ct))
}
