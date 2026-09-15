use crate::engine::structured::OpenAiNativeTool;
use crate::executive::error::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

/// OpenRouter `tool_choice` (ignored by Ollama / llama.cpp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolChoice {
    Auto,
    Required,
}

impl Serialize for ToolChoice {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            ToolChoice::Auto => "auto",
            ToolChoice::Required => "required",
        })
    }
}

/// Wire-level conversational role for a [`Message`].
///
/// `System` / `User` / `Assistant` are accepted by every chat template.
/// [`Role::Tool`] is the OpenAI-native tool-result role; OpenRouter emits it on the
/// chat stack after a native tool hop. Local backends map it to `user` if it ever
/// appears on their stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    /// Native tool-result frame (`role: "tool"` + `tool_call_id`). OpenRouter
    /// puts these on the chat stack after a native tool hop; local backends map
    /// the role to `user` if one ever appears.
    Tool,
}

impl Role {
    /// Canonical lowercase wire string (`"system"` / `"user"` / `"assistant"` / `"tool"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    /// Parse a wire string. Unknown roles fall back to [`Role::User`], preserving
    /// the historical `_ => MessageRole::User` behavior at the backends.
    pub fn from_wire(s: &str) -> Role {
        match s {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
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

/// One native tool call returned by a backend that supports OpenAI-style
/// `message.tool_calls`. Local backends always leave this empty; arguments stay
/// the raw JSON **string** until the orchestrator boundary parses them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineToolCall {
    /// Provider `tool_call_id`, required for the `role: "tool"` round-trip.
    pub id: Option<String>,
    pub name: String,
    /// Raw JSON string exactly as the provider returned it.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Provider `tool_call_id` on [`Role::Tool`] result frames.
    pub tool_call_id: Option<String>,
    /// Native tool calls on an assistant turn (OpenRouter round-trip).
    pub tool_calls: Vec<EngineToolCall>,
}

impl Message {
    /// Construct a message with empty tool metadata (the common case for all
    /// backends until the OpenRouter native round-trip is wired).
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    /// Construct a `system`-role message.
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    /// Construct a `user`-role message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    /// Construct an `assistant`-role message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }

    /// Construct a native `role: "tool"` result frame.
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: Vec::new(),
        }
    }

    /// Assistant turn that requested native tool calls.
    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<EngineToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls,
        }
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new(Role::User, String::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineResponse {
    pub content: String,
    /// Native tool calls; empty for Ollama / llama.cpp (and for OpenRouter talk turns).
    pub tool_calls: Vec<EngineToolCall>,
    /// Hosted reasoning trace (`message.reasoning`). Never mixed into `content`. Empty locally.
    pub reasoning: String,
    pub prompt_tokens: usize,
    pub generated_tokens: usize,
    /// Wall-clock ms for the completed request (streaming or non-streaming), for throughput metrics.
    pub generation_ms: u64,
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
    /// Mutually exclusive with [`Self::grammar_override`] by backend. Also mutually exclusive
    /// with [`Self::native_tools`] on a given OpenRouter request (tools win until a session
    /// downgrade to envelope `response_format`).
    pub response_json_schema: Option<Arc<serde_json::Value>>,
    /// OpenRouter native `tools[]` for this hop (same offered names as the schema subset).
    /// Ollama and llama.cpp ignore this. Empty/None means do not attach tools.
    pub native_tools: Option<Arc<Vec<OpenAiNativeTool>>>,
    /// OpenRouter `tool_choice`. Ignored unless [`Self::native_tools`] is attached.
    pub tool_choice: Option<ToolChoice>,
}

impl Default for LlmGenerateOptions {
    fn default() -> Self {
        Self {
            temperature: None,
            grammar_override: None,
            attach_session_grammar: true,
            response_json_schema: None,
            native_tools: None,
            tool_choice: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_tool_call_serde_round_trip() {
        let call = EngineToolCall {
            id: Some("call_abc".into()),
            name: "memory:query".into(),
            arguments: r#"{"query":"foo"}"#.into(),
        };
        let json = serde_json::to_string(&call).expect("serialize");
        let back: EngineToolCall = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(call, back);
    }

    #[test]
    fn engine_tool_call_serde_round_trip_without_id() {
        let call = EngineToolCall {
            id: None,
            name: "clock:now".into(),
            arguments: "{}".into(),
        };
        let json = serde_json::to_string(&call).expect("serialize");
        let back: EngineToolCall = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(call, back);
    }

    #[test]
    fn role_tool_wire_round_trip() {
        assert_eq!(Role::Tool.as_str(), "tool");
        assert_eq!(Role::from_wire("tool"), Role::Tool);
        assert_eq!(Role::from_wire("unknown"), Role::User);
    }

    #[test]
    fn engine_response_default_has_empty_tool_calls() {
        assert!(EngineResponse::default().tool_calls.is_empty());
    }
}
