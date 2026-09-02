//! Deterministic heuristics for when the runtime should nudge or pin plan tools.

use super::{load, PlanStepStatus, WorkingPlan};
use std::path::Path;

const RUNTIME_HINT_MULTI_STEP: &str = "[RUNTIME_HINT] User message looks multi-step; call plan:set (new mission) or plan:update (existing mission) BEFORE other tools unless the request is a trivial single tool. After each significant tool result, call plan:update (mark done, advance current_step_id, append short findings to scratch).";

const RUNTIME_HINT_ACTIVE_PLAN: &str = "[RUNTIME_HINT] Active working plan with open steps: execute the current step only, then plan:update (mark done, set current_step_id, scratch_append). Do not restart with plan:set unless the user changed the mission.";

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
pub async fn format_tui_summary(workspace_root: &Path) -> Option<String> {
    let plan = load(workspace_root).await.ok().flatten()?;
    format_plan_checklist(&plan)
}

/// Same formatting as [`format_tui_summary`] but synchronous when the plan is already loaded.
#[must_use]
pub fn format_plan_checklist(plan: &WorkingPlan) -> Option<String> {
    if plan.goal.trim().is_empty() && plan.steps.is_empty() {
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
}
