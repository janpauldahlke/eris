//! Shared OpenAI `/chat/completions` wire helpers for HTTP backends
//! ([`crate::engine::llama_cpp::LlamaCppClient`] and [`crate::engine::openrouter::OpenRouterClient`]).
//! Hosted models are just as strict about role ordering as local chat templates, so both
//! backends normalize through one copy — the shapes cannot drift apart.

use serde::Serialize;

/// One wire message for the OpenAI chat API.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatMsg {
    pub role: String,
    pub content: String,
}

/// Normalize messages for chat templates that require all system content at
/// the beginning (e.g. Qwen).  Merge leading consecutive system messages into
/// one; re-role any later system messages as "user" so the wire payload never
/// violates the "system-only-at-start" invariant.
pub(crate) fn normalize_system_messages(messages: Vec<ChatMsg>) -> Vec<ChatMsg> {
    if messages.is_empty() {
        return messages;
    }

    let leading_system_count = messages
        .iter()
        .take_while(|m| m.role == "system")
        .count();

    let mut out = Vec::with_capacity(messages.len());

    if leading_system_count > 1 {
        let merged: String = messages[..leading_system_count]
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        out.push(ChatMsg {
            role: "system".to_string(),
            content: merged,
        });
    } else if leading_system_count == 1 {
        out.push(ChatMsg {
            role: messages[0].role.clone(),
            content: messages[0].content.clone(),
        });
    }

    let mut had_stray = false;
    for m in messages.into_iter().skip(leading_system_count) {
        if m.role == "system" {
            had_stray = true;
            out.push(ChatMsg {
                role: "user".to_string(),
                content: format!("[System] {}", m.content),
            });
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
        if let Some(last) = out.last_mut() {
            if last.role == m.role {
                last.content.push_str(SEP);
                last.content.push_str(&m.content);
                coalesced_runs += 1;
                continue;
            }
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

/// Convert the engine-neutral stack into wire messages, applying both normalizations.
pub(crate) fn to_wire_messages(stack: &[crate::engine::Message]) -> Vec<ChatMsg> {
    let raw: Vec<ChatMsg> = stack
        .iter()
        .map(|m| ChatMsg {
            role: m.role.as_str().to_string(),
            content: m.content.clone(),
        })
        .collect();
    coalesce_consecutive_roles(normalize_system_messages(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Message, Role};

    fn sys(s: &str) -> ChatMsg {
        ChatMsg {
            role: "system".into(),
            content: s.into(),
        }
    }
    fn user(s: &str) -> ChatMsg {
        ChatMsg {
            role: "user".into(),
            content: s.into(),
        }
    }
    fn asst(s: &str) -> ChatMsg {
        ChatMsg {
            role: "assistant".into(),
            content: s.into(),
        }
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
            let out = normalize_system_messages(vec![
                sys("main"),
                sys("rolling summary"),
                user("hi"),
            ]);
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
                .map(|m| Message {
                    role: Role::from_wire(&m.role),
                    content: m.content.clone(),
                })
                .collect();
            let twice = project(&as_messages);
            let roles_once: Vec<&str> = once.iter().map(|m| m.role.as_str()).collect();
            let roles_twice: Vec<&str> = twice.iter().map(|m| m.role.as_str()).collect();
            assert_eq!(roles_once, roles_twice);
        }
    }
}
