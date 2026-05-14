//! Browse-cycle accounting once a **real Moltbook browse** has started.
//!
//! The ledger (and `[MOLTBOOK CYCLE — policy]` nudges) exists only after a successful
//! **`moltbook:home`**, **`moltbook:search`**, or **`moltbook:feed`** in the current user turn —
//! not from semantic tool routing, overlay latch, or unrelated tools such as `memory:query`.
//! This keeps private vault/memory work from inheriting public-swarm completion pressure.
//!
//! **Engagement (merge default, Workstream E):** conservative autonomy — after a successful
//! `moltbook:comments` read, the model should record **`moltbook:vote`** or **`memory:stage`**.
//! Autonomous `moltbook:comment` / `moltbook:post` remain human-gated; the ledger does not require them.

use crate::executive::error::FcpError;
use serde_json::Value;
use std::collections::HashSet;

/// Why autonomous browse could not meet engagement / read invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoltbookBlocker {
    ToolFault,
    AuthConfig,
    RemoteBodyParse,
    Network,
    Other,
}

#[derive(Debug, Default)]
pub struct MoltbookBrowseLedger {
    pub started_at_turn_seq: u64,
    pub home_ok: u32,
    pub search_ok: u32,
    pub feed_ok: u32,
    pub comments_ok: u32,
    pub votes: u32,
    pub comments_written: u32,
    pub posts: u32,
    pub memory_stage: u32,
    pub memory_commit: u32,
    pub remind_ok: u32,
    pub last_blocker: Option<MoltbookBlocker>,
    pub comments_unique_post_ids: HashSet<String>,
    /// Identical failing repeat-tool suppressions this step (telemetry).
    pub repeat_failure_suppressions: u32,
}

impl MoltbookBrowseLedger {
    pub fn new(turn_seq: u64) -> Self {
        Self {
            started_at_turn_seq: turn_seq,
            ..Default::default()
        }
    }

    pub fn record_repeat_failure_suppressed(&mut self) {
        self.repeat_failure_suppressions = self.repeat_failure_suppressions.saturating_add(1);
    }

    pub fn record_success(&mut self, tool_name: &str, args: &Value) {
        if tool_name.starts_with("moltbook:") {
            self.last_blocker = None;
        }
        match tool_name {
            "moltbook:home" => self.home_ok = self.home_ok.saturating_add(1),
            "moltbook:search" => self.search_ok = self.search_ok.saturating_add(1),
            "moltbook:feed" => self.feed_ok = self.feed_ok.saturating_add(1),
            "moltbook:comments" => {
                self.comments_ok = self.comments_ok.saturating_add(1);
                if let Some(pid) = args.get("post_id").and_then(|v| v.as_str()) {
                    let pid = pid.trim();
                    if !pid.is_empty() {
                        self.comments_unique_post_ids.insert(pid.to_string());
                    }
                }
            }
            "moltbook:vote" => self.votes = self.votes.saturating_add(1),
            "moltbook:comment" => self.comments_written = self.comments_written.saturating_add(1),
            "moltbook:post" => self.posts = self.posts.saturating_add(1),
            "memory:stage" => self.memory_stage = self.memory_stage.saturating_add(1),
            "memory:commit" | "memory:commit_all" => {
                self.memory_commit = self.memory_commit.saturating_add(1)
            }
            "agenda:remind_at" => self.remind_ok = self.remind_ok.saturating_add(1),
            _ => {}
        }
    }

    pub fn record_moltbook_tool_failure(&mut self, tool_name: &str, err: &FcpError) {
        if !tool_name.starts_with("moltbook:") {
            return;
        }
        self.last_blocker = Some(map_moltbook_blocker(err));
    }

    /// Single combined system line when browse invariants are not yet satisfied (no active blocker).
    pub fn missing_invariant_nudge(&self) -> Option<String> {
        if self.last_blocker.is_some() {
            return None;
        }
        let mut parts: Vec<&'static str> = Vec::new();
        let opened_thread = self.comments_ok >= 1;
        if !opened_thread {
            parts.push(
                "Open a thread: call `moltbook:comments` successfully on a `post_id` from `moltbook:home`, `moltbook:search`, or `moltbook:feed` before concluding this browse cycle.",
            );
        }
        if opened_thread && self.votes == 0 && self.memory_stage == 0 {
            parts.push(
                "Engagement (conservative): after opening comments, run `moltbook:vote` or `memory:stage`. Autonomous `moltbook:comment` / `moltbook:post` require explicit human approval.",
            );
        }
        if opened_thread
            && (self.votes >= 1 || self.memory_stage >= 1)
            && self.remind_ok == 0
        {
            parts.push(
                "Before wrapping this alarm cycle, schedule follow-up with `agenda:remind_at` unless the alarm is already expired.",
            );
        }
        if parts.is_empty() {
            None
        } else {
            Some(format!(
                "[MOLTBOOK CYCLE — policy] {}",
                parts.join(" ")
            ))
        }
    }
}

fn map_moltbook_blocker(err: &FcpError) -> MoltbookBlocker {
    match err {
        FcpError::MoltbookResponseParse(_) => MoltbookBlocker::RemoteBodyParse,
        FcpError::NetworkFault(_) => MoltbookBlocker::Network,
        FcpError::Config(_) => MoltbookBlocker::AuthConfig,
        FcpError::ToolFault { .. } => MoltbookBlocker::ToolFault,
        _ => MoltbookBlocker::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nudge_when_no_comments_yet() {
        let mut l = MoltbookBrowseLedger::new(1);
        l.record_success("moltbook:home", &json!({}));
        let n = l.missing_invariant_nudge().expect("comments floor");
        assert!(n.contains("moltbook:comments"));
    }

    #[test]
    fn no_engagement_nudge_when_blocker() {
        let mut l = MoltbookBrowseLedger::new(1);
        l.record_success("moltbook:comments", &json!({"post_id": "p1"}));
        l.last_blocker = Some(MoltbookBlocker::Network);
        assert!(l.missing_invariant_nudge().is_none());
    }

    #[test]
    fn engagement_nudge_after_comments_without_vote_or_stage() {
        let mut l = MoltbookBrowseLedger::new(1);
        l.record_success("moltbook:comments", &json!({"post_id": "p1"}));
        let n = l.missing_invariant_nudge().expect("engagement");
        assert!(n.contains("moltbook:vote") || n.contains("memory:stage"));
    }

    #[test]
    fn remind_nudge_after_engagement_met() {
        let mut l = MoltbookBrowseLedger::new(1);
        l.record_success("moltbook:comments", &json!({"post_id": "p1"}));
        l.record_success("moltbook:vote", &json!({}));
        let n = l.missing_invariant_nudge().expect("remind");
        assert!(n.contains("agenda:remind_at"));
    }

    #[test]
    fn no_nudge_when_all_invariants_met() {
        let mut l = MoltbookBrowseLedger::new(1);
        l.record_success("moltbook:comments", &json!({"post_id": "p1"}));
        l.record_success("memory:stage", &json!({}));
        l.record_success("agenda:remind_at", &json!({}));
        assert!(l.missing_invariant_nudge().is_none());
    }
}
