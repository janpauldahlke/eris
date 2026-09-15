use crate::executive::error::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Wire-level conversational role for a [`Message`].
///
/// This is intentionally limited to the three roles every chat template accepts.
/// Semantic distinctions (tool result vs. system directive vs. main prompt) are
/// classified at the backend **projection** boundary, not stored here — see
/// `crate::engine::projection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    /// Canonical lowercase wire string (`"system"` / `"user"` / `"assistant"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    /// Parse a wire string. Unknown roles fall back to [`Role::User`], preserving
    /// the historical `_ => MessageRole::User` behavior at the backends.
    pub fn from_wire(s: &str) -> Role {
        match s {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            _ => Role::User,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ergonomic comparison against wire strings so existing `msg.role == "system"`
/// call sites keep working after the `String` → [`Role`] migration.
impl PartialEq<&str> for Role {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for Role {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<Role> for &str {
    fn eq(&self, other: &Role) -> bool {
        *self == other.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    /// Construct a `system`-role message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Construct a `user`-role message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Construct an `assistant`-role message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineResponse {
    pub content: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    /// Wall-clock ms for the completed request (streaming or non-streaming), for throughput metrics.
    pub generation_ms: u64,
}

impl Default for EngineResponse {
    fn default() -> Self {
        Self {
            content: String::new(),
            prompt_tokens: 0,
            generated_tokens: 0,
            generation_ms: 0,
        }
    }
}

/// Optional knobs for a single [`LlmEngine::generate`] call (backends ignore unsupported fields).
#[derive(Debug, Clone, PartialEq)]
pub struct LlmGenerateOptions {
    /// When `Some`, overrides the backend default sampling temperature for this request only.
    pub temperature: Option<f32>,
    /// When `Some`, llama.cpp attaches this GBNF instead of the session grammar from [`crate::engine::llama_cpp::LlamaCppClient::set_grammar`].
    /// Ollama and other backends ignore this field.
    pub grammar_override: Option<Arc<str>>,
    /// When `false`, llama.cpp omits the `grammar` field from the HTTP request unless [`Self::grammar_override`] is set.
    /// Used for internal summarization passes that are not FCP agent JSON.
    pub attach_session_grammar: bool,
    /// When `Some`, OpenRouter sends `response_format: {type: "json_schema", strict: true, schema}`
    /// built from the same offered-tool set as the GBNF subset. Ollama and llama.cpp ignore this.
    /// Mutually exclusive with [`Self::grammar_override`] by backend.
    pub response_json_schema: Option<Arc<serde_json::Value>>,
}

impl Default for LlmGenerateOptions {
    fn default() -> Self {
        Self {
            temperature: None,
            grammar_override: None,
            attach_session_grammar: true,
            response_json_schema: None,
        }
    }
}

#[async_trait]
pub trait LlmEngine: Send + Sync {
    async fn generate(
        &self,
        stack: &[Message],
        available_tools_json: &str,
        stream_tx: Option<mpsc::UnboundedSender<String>>,
        options: LlmGenerateOptions,
    ) -> Result<EngineResponse>;
}
