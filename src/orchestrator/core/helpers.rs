use crate::engine::LlmEngine;
#[cfg(test)]
use crate::executive::error::FcpError;
#[cfg(test)]
use crate::orchestrator::r#loop::recovery_policy::{classify_tool_failure, ToolFailureAction};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::Orchestrator;

impl<E: LlmEngine> Orchestrator<E> {
    pub(super) const MAX_TOOL_RESULT_CHARS: usize = 2500;
    pub(super) const WEB_CONDENSATION_THRESHOLD: f32 = 0.85;

    pub(super) fn normalize_json(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut sorted = BTreeMap::new();
                for (k, v) in map {
                    sorted.insert(k.clone(), Self::normalize_json(v));
                }
                let mut normalized = serde_json::Map::new();
                for (k, v) in sorted {
                    normalized.insert(k, v);
                }
                serde_json::Value::Object(normalized)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::normalize_json).collect())
            }
            other => other.clone(),
        }
    }

    pub(super) fn agent_name(&self) -> String {
        let workspace_root = self
            .context_assembler
            .core_dir
            .parent()
            .unwrap_or(&self.context_assembler.core_dir);
        workspace_root
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("ERIS")
            .to_string()
    }

    pub(super) fn trim_chars(input: &str, max_len: usize) -> String {
        if input.len() <= max_len {
            return input.to_string();
        }
        let mut limit = max_len;
        while limit > 0 && !input.is_char_boundary(limit) {
            limit -= 1;
        }
        let mut out = input[..limit].to_string();
        out.push_str("… [truncated]");
        out
    }

    pub(super) fn last_user_content(&self) -> &str {
        self.chat_stack
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("")
    }

    /// Injected by `router` for agenda-linked alarms; must stay in sync with that format.
    pub(super) const AGENDA_CONFIRM_TASK_PREFIX: &'static str = "[AGENDA_CONFIRM task_id=";

    pub(super) fn extract_agenda_confirm_task_id(content: &str) -> Option<&str> {
        let idx = content.find(Self::AGENDA_CONFIRM_TASK_PREFIX)?;
        let start = idx + Self::AGENDA_CONFIRM_TASK_PREFIX.len();
        let rest = content.get(start..)?;
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ']')
            .unwrap_or(rest.len());
        let id = rest.get(..end)?.trim();
        if id.is_empty() {
            None
        } else {
            Some(id)
        }
    }

    /// Looks for a prior user line (excluding the latest user message) containing `AGENDA_CONFIRM`.
    pub(super) fn agenda_confirm_task_id_before_current_turn(
        stack: &[crate::engine::Message],
    ) -> Option<String> {
        let mut skipped_latest_user = false;
        for m in stack.iter().rev() {
            if m.role != "user" {
                continue;
            }
            if !skipped_latest_user {
                skipped_latest_user = true;
                continue;
            }
            if let Some(id) = Self::extract_agenda_confirm_task_id(&m.content) {
                return Some(id.to_string());
            }
        }
        None
    }

    /// Short explicit acknowledgments after an agenda alarm (avoid "yes" alone — too ambiguous).
    pub(super) fn user_text_means_agenda_done_ack(s: &str) -> bool {
        let t = s.trim();
        if t.is_empty() {
            return false;
        }
        let lower = t.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        match words.as_slice() {
            [w] => matches!(
                *w,
                "done" | "finished" | "complete" | "completed" | "yep" | "yeah" | "ok" | "okay"
            ),
            ["all", "done"] => true,
            ["did", "it"] => true,
            [a, "done"] if a.len() <= 4 => matches!(*a, "i'm" | "im" | "i"), // i'm done / i done (sloppy)
            _ => {
                lower == "task done"
                    || lower.starts_with("done ")
                    || lower.ends_with(" done")
                    || lower == "marked done"
            }
        }
    }

    pub(super) fn upsert_system_prompt(
        chat_stack: &mut Vec<crate::engine::Message>,
        prompt: String,
    ) {
        if let Some(first) = chat_stack.first_mut() {
            if first.role == "system" {
                first.content = prompt;
            } else {
                chat_stack.insert(0, crate::engine::Message {
                    role: "system".to_string(),
                    content: prompt,
                });
            }
        } else {
            chat_stack.push(crate::engine::Message {
                role: "system".to_string(),
                content: prompt,
            });
        }
    }

    pub(super) fn tool_fingerprint(name: &str, args: &serde_json::Value) -> String {
        let normalized = Self::normalize_json(args);
        let args_json = serde_json::to_string(&normalized).unwrap_or_else(|_| "null".to_string());
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
        hasher.update(args_json.as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(40);
        for b in &digest[..20] {
            let _ = write!(&mut hex, "{:02x}", b);
        }
        hex
    }

    #[cfg(test)]
    pub(super) fn is_schema_or_parse_tool_error(e: &FcpError) -> bool {
        matches!(
            classify_tool_failure(e, false),
            ToolFailureAction::TargetedSchemaRetry
        )
    }
}
