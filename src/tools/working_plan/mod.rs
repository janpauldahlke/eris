//! Working plan: structured mission state (goal, steps, scratch) persisted at
//! `.fcp/tools/working_plan.json` and re-injected into the LLM prompt each turn.
//!
//! Splits **mission state** from operator todos (`agenda.json`): the plan is what
//! the agent is *doing now*; the agenda is what the operator queued for later.

pub mod read;
pub mod set;
pub mod update;

pub use read::PlanReadTool;
pub use set::PlanSetTool;
pub use update::PlanUpdateTool;

use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use std::path::Path;
use tokio::fs;

use crate::executive::error::{FcpError, Result};

/// Default cap (chars) for the `[WORKING_PLAN]` prompt block when config is unset.
pub const DEFAULT_PROMPT_MAX_CHARS: usize = 1200;

/// How much of the scratch tail is rendered before the whole block is hard-capped.
const SCRATCH_TAIL_CHARS: usize = 400;

/// Number of upcoming steps rendered in the "Next:" line.
const NEXT_STEPS_SHOWN: usize = 5;

pub fn new_step_id() -> String {
    format!("{:x}", uuid::Uuid::new_v4().as_u128())
}

/// Lifecycle status of a single plan step.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    #[default]
    Pending,
    Active,
    Done,
    Skipped,
    Blocked,
}

impl PlanStepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanStepStatus::Pending => "pending",
            PlanStepStatus::Active => "active",
            PlanStepStatus::Done => "done",
            PlanStepStatus::Skipped => "skipped",
            PlanStepStatus::Blocked => "blocked",
        }
    }
}

/// Optional semantic tag for a step (model-filled, core stores only): lets
/// "assert weather and route are valid" appear as an explicit `validate` step
/// between `tool` steps without new tools.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    Tool,
    Validate,
    Clarify,
    HumanWait,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: PlanStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<PlanStepKind>,
}

/// Step input for `plan:set` steps and `plan:update` `steps_add`.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
pub struct PlanStepInput {
    /// Explicit id; auto-generated when omitted.
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub status: Option<PlanStepStatus>,
    #[serde(default)]
    pub kind: Option<PlanStepKind>,
}

/// On-disk shape of `.fcp/tools/working_plan.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct WorkingPlan {
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub current_step_id: Option<String>,
    #[serde(default)]
    pub scratch: String,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub version: u64,
}

impl WorkingPlan {
    /// Render the `[WORKING_PLAN]` prompt block body (goal/outcome, current step,
    /// next steps, scratch tail). Returns an empty string when there is nothing
    /// to inject; hard-capped at `max_chars` characters.
    pub fn render_prompt_block(&self, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let mut out = String::new();
        if !self.goal.trim().is_empty() {
            out.push_str(&format!("Goal: {}\n", self.goal.trim()));
        }
        if !self.outcome.trim().is_empty() {
            out.push_str(&format!("Outcome: {}\n", self.outcome.trim()));
        }

        if let Some(id) = self.current_step_id.as_deref() {
            if let Some(step) = self.steps.iter().find(|s| s.id == id) {
                out.push_str(&format!("Current: [{}] {}\n", step.id, step.title.trim()));
            }
        }

        let current = self.current_step_id.as_deref();
        let next: Vec<&PlanStep> = self
            .steps
            .iter()
            .filter(|s| {
                !current.is_some_and(|c| s.id == c)
                    && !matches!(s.status, PlanStepStatus::Done | PlanStepStatus::Skipped)
            })
            .take(NEXT_STEPS_SHOWN)
            .collect();
        if !next.is_empty() {
            let titles: Vec<String> =
                next.iter().map(|s| s.title.trim().to_string()).collect();
            out.push_str(&format!("Next: {}\n", titles.join(" | ")));
        }

        let scratch_tail = tail_chars(&self.scratch, SCRATCH_TAIL_CHARS);
        if !scratch_tail.trim().is_empty() {
            out.push_str(&format!("Scratch: {}", scratch_tail.trim()));
        }

        if out.is_empty() {
            return String::new();
        }

        let mut block = out;
        if block.chars().count() > max_chars {
            const MARKER: &str = "...[truncated]";
            if max_chars >= MARKER.chars().count() + 1 {
                let keep = max_chars - MARKER.chars().count();
                let mut kept: String = block.chars().take(keep).collect();
                kept.push_str(MARKER);
                block = kept;
            } else {
                block = block.chars().take(max_chars).collect();
            }
        }
        block
    }

    /// All steps except `done`/`skipped`, in order.
    pub fn open_steps(&self) -> Vec<&PlanStep> {
        self.steps
            .iter()
            .filter(|s| !matches!(s.status, PlanStepStatus::Done | PlanStepStatus::Skipped))
            .collect()
    }
}

/// Last up to `n` characters of `s` (char-boundary safe).
fn tail_chars(s: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let total = s.chars().count();
    if total <= n {
        return s.to_string();
    }
    let skip = total - n;
    s.chars().skip(skip).collect()
}

/// Load the working plan from disk; `None` when the file is missing or empty.
pub async fn load(workspace_root: &Path) -> Result<Option<WorkingPlan>> {
    let path = crate::vault_layout::working_plan_json(workspace_root);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).await.map_err(FcpError::Io)?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let plan: WorkingPlan = serde_json::from_str(&content).map_err(FcpError::ParseFault)?;
    Ok(Some(plan))
}

/// Persist the working plan (pretty JSON, directory created if needed).
pub async fn save(workspace_root: &Path, plan: &WorkingPlan) -> Result<()> {
    let path = crate::vault_layout::working_plan_json(workspace_root);
    let content = serde_json::to_string_pretty(plan).map_err(FcpError::ParseFault)?;
    fs::create_dir_all(crate::vault_layout::tools_dir(workspace_root))
        .await
        .map_err(FcpError::Io)?;
    fs::write(&path, content).await.map_err(FcpError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, title: &str, status: PlanStepStatus) -> PlanStep {
        PlanStep {
            id: id.to_string(),
            title: title.to_string(),
            status,
            kind: None,
        }
    }

    #[test]
    fn render_prompt_block_empty_plan_is_empty_string() {
        let plan = WorkingPlan::default();
        assert_eq!(plan.render_prompt_block(DEFAULT_PROMPT_MAX_CHARS), "");
    }

    #[test]
    fn render_prompt_block_renders_goal_outcome_current_and_next() {
        let plan = WorkingPlan {
            goal: "Ship the slice".into(),
            outcome: "Tests green".into(),
            steps: vec![
                step("a", "Read doc", PlanStepStatus::Done),
                step("b", "Implement tools", PlanStepStatus::Active),
                step("c", "Write tests", PlanStepStatus::Pending),
                step("d", "Skip legacy", PlanStepStatus::Skipped),
                step("e", "Review", PlanStepStatus::Pending),
                step("f", "Ship", PlanStepStatus::Pending),
                step("g", "Announce", PlanStepStatus::Pending),
            ],
            current_step_id: Some("b".into()),
            scratch: "note one".into(),
            updated_at: 1,
            version: 1,
        };
        let block = plan.render_prompt_block(DEFAULT_PROMPT_MAX_CHARS);
        assert!(block.contains("Goal: Ship the slice"), "block: {block}");
        assert!(block.contains("Outcome: Tests green"), "block: {block}");
        assert!(block.contains("Current: [b] Implement tools"), "block: {block}");
        // Next: open steps after current, capped at NEXT_STEPS_SHOWN; done/skipped omitted
        assert!(block.contains("Next: "), "block: {block}");
        assert!(!block.contains("Read doc"), "done step leaked: {block}");
        assert!(!block.contains("Skip legacy"), "skipped step leaked: {block}");
        assert!(block.contains("Scratch: note one"), "block: {block}");
    }

    #[test]
    fn render_prompt_block_respects_char_cap() {
        let plan = WorkingPlan {
            goal: "g".into(),
            outcome: String::new(),
            steps: vec![step("a", "A".into(), PlanStepStatus::Pending)],
            current_step_id: Some("a".into()),
            scratch: "x".repeat(5000),
            updated_at: 0,
            version: 0,
        };
        let block = plan.render_prompt_block(200);
        assert!(
            block.chars().count() <= 200,
            "block too long: {}",
            block.chars().count()
        );
        assert!(block.ends_with("...[truncated]"), "block: {block}");
        // Cap smaller than the truncation marker must still be honored.
        let tiny = plan.render_prompt_block(3);
        assert!(tiny.chars().count() <= 3);
    }

    #[test]
    fn render_prompt_block_zero_cap_is_empty() {
        let plan = WorkingPlan {
            goal: "g".into(),
            ..Default::default()
        };
        assert_eq!(plan.render_prompt_block(0), "");
    }

    #[test]
    fn roundtrip_serde_preserves_plan() {
        let plan = WorkingPlan {
            goal: "g".into(),
            outcome: "o".into(),
            steps: vec![PlanStep {
                id: "a".into(),
                title: "A".into(),
                status: PlanStepStatus::Blocked,
                kind: Some(PlanStepKind::HumanWait),
            }],
            current_step_id: Some("a".into()),
            scratch: "s".into(),
            updated_at: 42,
            version: 7,
        };
        let s = serde_json::to_string(&plan).unwrap();
        let back: WorkingPlan = serde_json::from_str(&s).unwrap();
        assert_eq!(back.version, 7);
        assert_eq!(back.updated_at, 42);
        assert_eq!(back.steps[0].kind, Some(PlanStepKind::HumanWait));
        assert_eq!(back.steps[0].status, PlanStepStatus::Blocked);
    }

    #[test]
    fn serde_status_and_kind_use_snake_case() {
        assert_eq!(
            serde_json::to_value(PlanStepStatus::Pending).unwrap(),
            serde_json::json!("pending")
        );
        assert_eq!(
            serde_json::to_value(PlanStepKind::HumanWait).unwrap(),
            serde_json::json!("human_wait")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_save_roundtrip_on_disk() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let plan = WorkingPlan {
            goal: "g".into(),
            version: 1,
            ..Default::default()
        };
        save(dir.path(), &plan).await?;
        assert!(crate::vault_layout::working_plan_json(dir.path()).exists());
        let back = load(dir.path()).await?;
        let back = back.expect("plan should load");
        assert_eq!(back.goal, "g");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_missing_file_is_none() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).await?.is_none());
        Ok(())
    }
}
