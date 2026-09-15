//! Shared OpenAI `/chat/completions` wire helpers for HTTP backends
//! ([`crate::engine::llama_cpp::LlamaCppClient`] and [`crate::engine::openrouter::OpenRouterClient`]).
//! Hosted models are just as strict about role ordering as local chat templates, so both
//! backends normalize through one copy — the shapes cannot drift apart.
//!
//! Native `role: "tool"` frames and assistant `tool_calls` serialize here (skipping when
//! empty so llama.cpp payloads stay byte-identical). The orchestrator puts these on the
//! chat stack for OpenRouter native tool hops; local backends keep folded `system` results.

use serde::Serialize;

use crate::engine::EngineToolCall;

/// One wire message for the OpenAI chat API.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatMsg {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ChatToolCall>,
}

impl ChatMsg {
    pub(crate) fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

/// OpenAI `message.tool_calls[]` item (`type: "function"`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ChatToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ChatToolCallFunction {
    pub name: String,
    pub arguments: String,
}

impl ChatToolCall {
    fn from_engine(call: &EngineToolCall) -> Self {
        Self {
            id: call.id.clone(),
            kind: "function".to_string(),
            function: ChatToolCallFunction {
                name: call.name.clone(),
                arguments: sanitize_tool_call_arguments(&call.arguments),
            },
        }
    }
}

/// Keep the first JSON object in a tool-call `arguments` string.
///
/// Gateways that omit or reuse `tool_calls[].index` can concatenate two objects.
/// Echoing that blob on the next turn makes LiteLLM `json.loads` raise
/// `Extra data: line 1 column N`.
pub(crate) fn sanitize_tool_call_arguments(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<serde_json::Value>();
    match stream.next() {
        Some(Ok(serde_json::Value::Object(map))) => match stream.next() {
            None => trimmed.to_string(),
            Some(_) => {
                tracing::warn!(
                    "tool-call arguments contained trailing JSON; keeping the first object"
                );
                serde_json::Value::Object(map).to_string()
            }
        },
        Some(Ok(_)) => {
            tracing::warn!("tool-call arguments were JSON but not an object; using empty object");
            "{}".to_string()
        }
        Some(Err(e)) => {
            tracing::warn!(
                error = %e,
                "tool-call arguments JSON parse failed; using empty object"
            );
            "{}".to_string()
        }
        None => "{}".to_string(),
    }
}

/// Normalize messages for chat templates that require all system content at
/// the beginning (e.g. Qwen).  Merge leading consecutive system messages into
/// one; re-role any later system messages as "user" so the wire payload never
/// violates the "system-only-at-start" invariant.
pub(crate) fn normalize_system_messages(messages: Vec<ChatMsg>) -> Vec<ChatMsg> {
    if messages.is_empty() {
        return messages;
    }

    let leading_system_count = messages.iter().take_while(|m| m.role == "system").count();

    let mut out = Vec::with_capacity(messages.len());

    if leading_system_count > 1 {
        let merged: String = messages[..leading_system_count]
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        out.push(ChatMsg::new("system", merged));
    } else if leading_system_count == 1 {
        out.push(messages[0].clone());
    }

    let mut had_stray = false;
    for m in messages.into_iter().skip(leading_system_count) {
        if m.role == "system" {
            had_stray = true;
            out.push(ChatMsg::new("user", format!("[System] {}", m.content)));
        } else {
            out.push(m);
        }
    }

    if had_stray {
        tracing::warn!(
            "openai_wire: stray system messages after non-system rows re-roled as user for strict chat template"
        );
    }

    out
}

/// Coalesce consecutive same-role wire messages into one, guaranteeing strict
/// `user`/`assistant` alternation after the leading system block.
///
/// This is the long-context fix from main (see
/// `docs/TODO/REFACTOR_LLAMACPP_CONTEXT_HANDLING.md`). After
/// [`normalize_system_messages`] has folded stray `system` rows (tool results,
/// directives) into `[System] …` **user** turns, a long tool-heavy session
/// contains many *consecutive* `user` turns — a shape no chat template was
/// trained on. Merging adjacent same-role turns restores clean alternation
/// without dropping content.
///
/// It also subsumes the older trailing-assistant merge: OpenAI-compatible
/// servers reject two or more `assistant` messages at the tail, and coalescing
/// collapses any run of assistant rows (trailing or interior) into one wire message.
pub(crate) fn coalesce_consecutive_roles(messages: Vec<ChatMsg>) -> Vec<ChatMsg> {
    const SEP: &str = "\n\n";
    let mut out: Vec<ChatMsg> = Vec::with_capacity(messages.len());
    let mut coalesced_runs = 0usize;
    for m in messages {
        if let Some(last) = out.last_mut()
            && can_coalesce(last, &m)
        {
            last.content.push_str(SEP);
            last.content.push_str(&m.content);
            coalesced_runs += 1;
            continue;
        }
        out.push(m);
    }
    if coalesced_runs > 0 {
        tracing::debug!(
            coalesced_runs,
            wire_messages = out.len(),
            "openai_wire: coalesced consecutive same-role turns for clean template alternation"
        );
    }
    out
}

/// Consecutive `tool` frames each carry a distinct `tool_call_id` and must not be
/// merged. Assistant turns that already carry native `tool_calls` are also left
/// alone so the provider can correlate results.
fn can_coalesce(last: &ChatMsg, next: &ChatMsg) -> bool {
    last.role == next.role
        && last.role != "tool"
        && last.tool_calls.is_empty()
        && next.tool_calls.is_empty()
        && last.tool_call_id.is_none()
        && next.tool_call_id.is_none()
}

/// Convert the engine-neutral stack into wire messages, applying both normalizations.
pub(crate) fn to_wire_messages(stack: &[crate::engine::Message]) -> Vec<ChatMsg> {
    let raw: Vec<ChatMsg> = stack
        .iter()
        .map(|m| ChatMsg {
            role: m.role.as_str().to_string(),
            content: m.content.clone(),
            tool_call_id: m.tool_call_id.clone(),
            tool_calls: m.tool_calls.iter().map(ChatToolCall::from_engine).collect(),
        })
        .collect();
    coalesce_consecutive_roles(normalize_system_messages(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Message, Role};

    fn sys(s: &str) -> ChatMsg {
        ChatMsg::new("system", s)
    }
    fn user(s: &str) -> ChatMsg {
        ChatMsg::new("user", s)
    }
    fn asst(s: &str) -> ChatMsg {
        ChatMsg::new("assistant", s)
    }

    #[test]
    fn sanitize_keeps_a_single_object_verbatim() {
        assert_eq!(
            sanitize_tool_call_arguments(r#"{"query":"foo"}"#),
            r#"{"query":"foo"}"#
        );
        assert_eq!(sanitize_tool_call_arguments("  "), "{}");
    }

    #[test]
    fn sanitize_keeps_first_object_when_gateway_concatenated_two() {
        let raw = r#"{"query":"who am I"}{"limit":10}"#;
        let out = sanitize_tool_call_arguments(raw);
        let value: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["query"], "who am I");
        assert!(value.get("limit").is_none());
        serde_json::from_str::<serde_json::Value>(&out).expect("must be a single JSON value");
    }

    mod normalize_system_messages_tests {
        use super::*;

        #[test]
        fn empty_stack_unchanged() {
            let out = normalize_system_messages(vec![]);
            assert!(out.is_empty());
        }

        #[test]
        fn single_system_at_front_unchanged() {
            let out = normalize_system_messages(vec![sys("prompt"), user("hi")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "system");
            assert_eq!(out[0].content, "prompt");
            assert_eq!(out[1].role, "user");
        }

        #[test]
        fn multiple_leading_systems_merged() {
            let out =
                normalize_system_messages(vec![sys("main"), sys("rolling summary"), user("hi")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "system");
            assert!(out[0].content.contains("main"));
            assert!(out[0].content.contains("rolling summary"));
            assert_eq!(out[1].role, "user");
        }

        #[test]
        fn stray_system_after_user_reroled() {
            let out = normalize_system_messages(vec![
                sys("prompt"),
                user("hello"),
                asst("hi back"),
                sys("Tool 'x:y' succeeded: data"),
            ]);
            assert_eq!(out.len(), 4);
            assert_eq!(out[0].role, "system");
            assert_eq!(out[3].role, "user");
            assert!(out[3].content.starts_with("[System]"));
            assert!(out[3].content.contains("Tool 'x:y' succeeded: data"));
        }

        #[test]
        fn realistic_tool_turn_stack() {
            let out = normalize_system_messages(vec![
                sys("prompt"),
                user("weather?"),
                asst("{tool_calls: ...}"),
                sys("Tool 'weather:get' succeeded: 25°C"),
                sys("POST_TOOL_GUIDANCE"),
                sys("JIT guidance"),
            ]);
            assert_eq!(out[0].role, "system");
            assert_eq!(out[0].content, "prompt");
            for m in &out[1..] {
                assert_ne!(m.role, "system", "no system messages after index 0");
            }
            assert_eq!(out[3].role, "user");
            assert!(out[3].content.contains("weather:get"));
        }

        #[test]
        fn no_system_messages_at_all() {
            let out = normalize_system_messages(vec![user("hi"), asst("hello")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "user");
            assert_eq!(out[1].role, "assistant");
        }
    }

    mod coalesce_and_fold_projection_tests {
        use super::*;

        /// The full OpenAI-wire projection as applied in `to_wire_messages`.
        fn project(stack: &[Message]) -> Vec<ChatMsg> {
            to_wire_messages(stack)
        }

        /// Invariant helper: after the (optional) single leading system message,
        /// no two adjacent turns share a role, and no `system` appears past index 0.
        fn assert_clean_alternation(out: &[ChatMsg]) {
            for (i, m) in out.iter().enumerate() {
                if i > 0 {
                    assert_ne!(m.role, "system", "system message past index 0 at {i}");
                }
            }
            for w in out.windows(2) {
                assert_ne!(w[0].role, w[1].role, "adjacent same-role turns: {:?}", w);
            }
        }

        #[test]
        fn empty_and_single_unchanged() {
            assert!(coalesce_consecutive_roles(vec![]).is_empty());
            let out = coalesce_consecutive_roles(vec![asst("only")]);
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].content, "only");
        }

        #[test]
        fn consecutive_users_coalesced() {
            let out = coalesce_consecutive_roles(vec![user("a"), user("b"), asst("c")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].role, "user");
            assert!(out[0].content.contains('a') && out[0].content.contains('b'));
            assert_eq!(out[1].role, "assistant");
        }

        #[test]
        fn trailing_assistants_collapsed() {
            let out = coalesce_consecutive_roles(vec![user("u"), asst("x"), asst("y"), asst("z")]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[1].role, "assistant");
            assert!(out[1].content.contains('x'));
            assert!(out[1].content.contains('y'));
            assert!(out[1].content.contains('z'));
        }

        #[test]
        fn tool_heavy_session_stays_alternating_and_lossless() {
            // Mimics a long agent loop: assistant tool-call, then several system
            // rows (tool result + directives), repeated.
            let stack = vec![
                Message::system("MAIN PROMPT"),
                Message::user("weather in berlin and paris?"),
                Message::assistant("{\"tool_calls\":[{\"name\":\"weather:get\"}]}"),
                Message::system("Tool 'weather:get' succeeded: Berlin 25C"),
                Message::system("[SYSTEM] cap note"),
                Message::assistant("{\"tool_calls\":[{\"name\":\"weather:get\"}]}"),
                Message::system("Tool 'weather:get' succeeded: Paris 22C"),
                Message::assistant("{\"message_to_user\":\"Berlin 25C, Paris 22C\"}"),
            ];
            let out = project(&stack);
            assert_eq!(out[0].role, "system");
            assert!(out[0].content.contains("MAIN PROMPT"));
            assert_clean_alternation(&out);
            // No tool-result content is lost anywhere in the wire payload.
            let joined = out.iter().map(|m| m.content.as_str()).collect::<String>();
            assert!(joined.contains("Berlin 25C"));
            assert!(joined.contains("Paris 22C"));
            assert!(joined.contains("cap note"));
        }

        #[test]
        fn projection_is_idempotent_in_shape() {
            let stack = vec![
                Message::system("S"),
                Message::user("u"),
                Message::assistant("a"),
                Message::system("tool ok"),
                Message::system("more tool ok"),
                Message::assistant("a2"),
            ];
            let once = project(&stack);
            // Re-projecting the wire result (as if it were the canonical stack) must
            // not further change the role structure.
            let as_messages: Vec<Message> = once
                .iter()
                .map(|m| {
                    let mut msg = Message::new(Role::from_wire(&m.role), m.content.clone());
                    msg.tool_call_id = m.tool_call_id.clone();
                    msg
                })
                .collect();
            let twice = project(&as_messages);
            let roles_once: Vec<&str> = once.iter().map(|m| m.role.as_str()).collect();
            let roles_twice: Vec<&str> = twice.iter().map(|m| m.role.as_str()).collect();
            assert_eq!(roles_once, roles_twice);
        }

        #[test]
        fn tool_role_emits_tool_frame_with_call_id() {
            use crate::engine::EngineToolCall;
            let stack = vec![
                Message::system("S"),
                Message::user("u"),
                Message::assistant_with_tool_calls(
                    "",
                    vec![EngineToolCall {
                        id: Some("call_1".into()),
                        name: "memory:query".into(),
                        arguments: r#"{"query":"x"}"#.into(),
                    }],
                ),
                Message::tool("result body", "call_1"),
            ];
            let out = project(&stack);
            let tool = out
                .iter()
                .find(|m| m.role == "tool")
                .expect("native tool frame");
            assert_eq!(tool.tool_call_id.as_deref(), Some("call_1"));
            assert_eq!(tool.content, "result body");
            let assistant = out
                .iter()
                .find(|m| m.role == "assistant")
                .expect("assistant with tool_calls");
            assert_eq!(assistant.tool_calls.len(), 1);
            assert_eq!(assistant.tool_calls[0].id.as_deref(), Some("call_1"));
            assert_eq!(assistant.tool_calls[0].function.name, "memory:query");
            assert_eq!(assistant.tool_calls[0].kind, "function");
        }

        #[test]
        fn concatenated_tool_arguments_are_sanitized_on_the_wire() {
            use crate::engine::EngineToolCall;
            let stack = vec![
                Message::system("S"),
                Message::user("u"),
                Message::assistant_with_tool_calls(
                    "",
                    vec![EngineToolCall {
                        id: Some("call_1".into()),
                        name: "vault:search".into(),
                        arguments: r#"{"query":"who am I"}{"limit":10}"#.into(),
                    }],
                ),
            ];
            let out = project(&stack);
            let assistant = out
                .iter()
                .find(|m| m.role == "assistant")
                .expect("assistant");
            let args = &assistant.tool_calls[0].function.arguments;
            serde_json::from_str::<serde_json::Value>(args).expect("single JSON value");
            let value: serde_json::Value = serde_json::from_str(args).expect("json");
            assert_eq!(value["query"], "who am I");
            assert!(value.get("limit").is_none());
        }

        #[test]
        fn consecutive_tool_frames_are_not_coalesced() {
            let stack = vec![
                Message::system("S"),
                Message::user("u"),
                Message::assistant("tool batch"),
                Message::tool("first", "call_a"),
                Message::tool("second", "call_b"),
            ];
            let out = project(&stack);
            let tools: Vec<&ChatMsg> = out.iter().filter(|m| m.role == "tool").collect();
            assert_eq!(tools.len(), 2, "each tool result stays its own wire frame");
            assert_eq!(tools[0].tool_call_id.as_deref(), Some("call_a"));
            assert_eq!(tools[1].tool_call_id.as_deref(), Some("call_b"));
        }

        #[test]
        fn empty_tool_metadata_omitted_from_json() {
            let json = serde_json::to_value(ChatMsg::new("user", "hi")).expect("serialize");
            assert_eq!(json, serde_json::json!({"role": "user", "content": "hi"}));
        }

        #[test]
        fn post_tool_guidance_after_native_tools_stays_off_tool_frames() {
            use crate::engine::EngineToolCall;
            let stack = vec![
                Message::system("S"),
                Message::user("u"),
                Message::assistant_with_tool_calls(
                    "",
                    vec![
                        EngineToolCall {
                            id: Some("call_a".into()),
                            name: "memory:query".into(),
                            arguments: r#"{"query":"x"}"#.into(),
                        },
                        EngineToolCall {
                            id: Some("call_b".into()),
                            name: "clock:now".into(),
                            arguments: "{}".into(),
                        },
                    ],
                ),
                Message::tool("first", "call_a"),
                Message::tool("second", "call_b"),
                Message::system("POST_TOOL_GUIDANCE"),
            ];
            let out = project(&stack);
            let tools: Vec<&ChatMsg> = out.iter().filter(|m| m.role == "tool").collect();
            assert_eq!(tools.len(), 2);
            assert_eq!(tools[0].tool_call_id.as_deref(), Some("call_a"));
            assert_eq!(tools[1].tool_call_id.as_deref(), Some("call_b"));
            let guidance = out
                .iter()
                .find(|m| m.content.contains("POST_TOOL_GUIDANCE"))
                .expect("guidance");
            assert_eq!(guidance.role, "user");
            assert_ne!(guidance.role, "tool");
        }
    }
}
