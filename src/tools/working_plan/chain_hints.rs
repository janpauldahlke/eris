//! Deterministic heuristics for when the runtime should nudge or pin plan tools.

use super::{load, PlanStepStatus, WorkingPlan};
use std::path::Path;

const RUNTIME_HINT_MULTI_STEP: &str = "[RUNTIME_HINT] User message looks multi-step; call plan:set (new mission) or plan:update (existing mission) BEFORE other tools unless the request is a trivial single tool. After each significant tool result, call plan:advance (or plan:update: mark done + set current_step_id + scratch_append). When finished, plan:clear or let auto-archive run when all steps are done.";

const RUNTIME_HINT_ACTIVE_PLAN: &str = "[RUNTIME_HINT] Active working plan with open steps: the slim tool offer is scoped to the CURRENT step (re-ranked each tool round). Execute that step's tool(s) only, then plan:advance (or plan:update). Do not steps_add a title that already exists. Do not restart with plan:set unless the user changed the mission. Call plan:clear if abandoning.";

/// Extract `domain:verb` tokens from text that appear in `registered` (order preserved).
#[must_use]
pub fn extract_registered_tool_refs(text: &str, registered: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = None;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match start {
            None => {
                if c.is_ascii_lowercase() {
                    start = Some(i);
                }
                i += 1;
            }
            Some(s) => {
                let is_name_char = c.is_ascii_lowercase()
                    || c.is_ascii_digit()
                    || c == '_'
                    || c == ':';
                if is_name_char {
                    i += 1;
                } else {
                    push_tool_candidate(&text[s..i], registered, &mut out);
                    start = None;
                    i += 1;
                }
            }
        }
    }
    if let Some(s) = start {
        push_tool_candidate(&text[s..], registered, &mut out);
    }
    out
}

fn push_tool_candidate(token: &str, registered: &[String], out: &mut Vec<String>) {
    if !token.contains(':') {
        return;
    }
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() != 2 {
        return;
    }
    if parts[0].is_empty() || parts[1].is_empty() {
        return;
    }
    if !parts[0]
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return;
    }
    if !parts[1]
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return;
    }
    if registered.iter().any(|n| n == token) && !out.iter().any(|n| n == token) {
        out.push(token.to_string());
    }
}

/// Seed text for re-ranking the slim tool offer to the current plan step.
#[must_use]
pub fn current_step_offer_seed(plan: &WorkingPlan) -> Option<String> {
    let step = plan.current_step()?;
    let title = step.title.trim();
    if title.is_empty() {
        return None;
    }
    Some(title.to_string())
}

/// Merge explicit title tool refs ahead of semantic ranks (deduped).
#[must_use]
pub fn merge_step_offer_seeds(explicit: Vec<String>, ranked: Vec<String>) -> Vec<String> {
    let mut out = explicit;
    for name in ranked {
        if !out.iter().any(|n| n == &name) {
            out.push(name);
        }
    }
    out
}

/// Lightweight scan of the last user message for multi-step / chained-workflow markers.
#[must_use]
pub fn user_message_suggests_plan(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();

    let phrase_hits = [
        " then ",
        " after ",
        " first ",
        " next ",
        " and then ",
        " and also ",
        " step by step",
        " in order",
        "do these in order",
        "do this in order",
        " numbered list",
        "validate then",
        "check then",
        " before you ",
    ]
    .iter()
    .filter(|m| lower.contains(*m))
    .count();

    if phrase_hits >= 1 {
        return true;
    }

    if text.lines().any(|line| line_has_numbered_prefix(line.trim())) {
        return true;
    }

    // Two or more semicolon-separated clauses often imply a chain.
    if text.matches(';').count() >= 2 {
        return true;
    }

    false
}

fn line_has_numbered_prefix(line: &str) -> bool {
    if line.len() < 2 {
        return false;
    }
    let mut chars = line.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_digit() {
        return false;
    }
    matches!(chars.next(), Some('.' | ')'))
}

/// Whether `.fcp/tools/working_plan.json` exists with at least one open step.
pub async fn has_open_working_plan(workspace_root: &Path) -> bool {
    match load(workspace_root).await {
        Ok(Some(plan)) => !plan.open_steps().is_empty(),
        _ => false,
    }
}

/// Fixed prompt line(s) appended next to `[WORKING_PLAN]` when hints are enabled.
#[must_use]
pub fn runtime_hint_block(multi_step_message: bool, active_open_plan: bool) -> Option<String> {
    match (multi_step_message, active_open_plan) {
        (true, true) => Some(format!(
            "{RUNTIME_HINT_MULTI_STEP}\n{RUNTIME_HINT_ACTIVE_PLAN}"
        )),
        (true, false) => Some(RUNTIME_HINT_MULTI_STEP.to_string()),
        (false, true) => Some(RUNTIME_HINT_ACTIVE_PLAN.to_string()),
        (false, false) => None,
    }
}

/// Compact checklist for TUI Status / web `active_task` (reuses presentation field).
/// Hidden when the plan has no open steps (completed / abandoned clutter).
pub async fn format_tui_summary(workspace_root: &Path) -> Option<String> {
    let plan = load(workspace_root).await.ok().flatten()?;
    if plan.open_steps().is_empty() {
        return None;
    }
    format_plan_checklist(&plan)
}

/// Same formatting as [`format_tui_summary`] but synchronous when the plan is already loaded.
#[must_use]
pub fn format_plan_checklist(plan: &WorkingPlan) -> Option<String> {
    if plan.goal.trim().is_empty() && plan.steps.is_empty() {
        return None;
    }
    // Operator chrome: only show while a mission is still in flight.
    if plan.open_steps().is_empty() {
        return None;
    }

    let current = plan.current_step_id.as_deref();
    let mut lines: Vec<String> = Vec::new();

    if !plan.goal.trim().is_empty() {
        let goal = plan.goal.trim();
        let short = if goal.chars().count() > 72 {
            format!("{}…", goal.chars().take(71).collect::<String>())
        } else {
            goal.to_string()
        };
        lines.push(format!("Plan: {short}"));
    }

    for step in &plan.steps {
        let marker = step_marker(step, current);
        lines.push(format!("{marker} {}", step.title.trim()));
    }

    if lines.len() <= 1 && plan.steps.is_empty() {
        return None;
    }

    Some(lines.join("\n"))
}

fn step_marker(step: &super::PlanStep, current: Option<&str>) -> &'static str {
    match step.status {
        PlanStepStatus::Done | PlanStepStatus::Skipped => "[x]",
        PlanStepStatus::Blocked => "[!]",
        PlanStepStatus::Active if current == Some(step.id.as_str()) => "[>]",
        PlanStepStatus::Pending if current == Some(step.id.as_str()) => "[>]",
        _ => "[ ]",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_then_chain() {
        assert!(user_message_suggests_plan(
            "Get weather in Berlin then search for AI news"
        ));
    }

    #[test]
    fn detects_numbered_list() {
        assert!(user_message_suggests_plan(
            "Do these in order:\n1) Tell me the time\n2) List vault"
        ));
    }

    #[test]
    fn ignores_single_shot() {
        assert!(!user_message_suggests_plan("What time is it?"));
    }

    #[test]
    fn runtime_hint_active_plan_only() {
        let block = runtime_hint_block(false, true).expect("hint");
        assert!(block.contains("Active working plan"));
    }

    #[test]
    fn checklist_hides_when_no_open_steps() {
        let plan = WorkingPlan {
            goal: "Done mission".into(),
            steps: vec![super::super::PlanStep {
                id: "a".into(),
                title: "Only".into(),
                status: PlanStepStatus::Done,
                kind: None,
            }],
            current_step_id: None,
            ..Default::default()
        };
        assert!(format_plan_checklist(&plan).is_none());
    }

    #[test]
    fn checklist_marks_current_step() {
        let plan = WorkingPlan {
            goal: "Test mission".into(),
            steps: vec![
                super::super::PlanStep {
                    id: "a".into(),
                    title: "First".into(),
                    status: PlanStepStatus::Done,
                    kind: None,
                },
                super::super::PlanStep {
                    id: "b".into(),
                    title: "Second".into(),
                    status: PlanStepStatus::Active,
                    kind: None,
                },
            ],
            current_step_id: Some("b".into()),
            ..Default::default()
        };
        let text = format_plan_checklist(&plan).expect("checklist");
        assert!(text.contains("[x] First"));
        assert!(text.contains("[>] Second"));
    }

    #[test]
    fn extracts_registered_tool_refs_from_title() {
        let registered = vec![
            "clock:now".into(),
            "wiki:summary".into(),
            "vault:list".into(),
        ];
        let found = extract_registered_tool_refs(
            "1) clock:now — exact time then wiki:summary for Rust",
            &registered,
        );
        assert_eq!(
            found,
            vec!["clock:now".to_string(), "wiki:summary".to_string()]
        );
    }

    #[test]
    fn merge_step_seeds_puts_explicit_first() {
        let merged = merge_step_offer_seeds(
            vec!["clock:now".into()],
            vec!["vault:list".into(), "clock:now".into()],
        );
        assert_eq!(
            merged,
            vec!["clock:now".to_string(), "vault:list".to_string()]
        );
    }
}
