use std::collections::HashSet;
use std::path::Path;

use crate::executive::error::Result;

use super::store::load_vault_skill_by_id;
use super::types::SkillPriority;

const MAX_SKILL_CANDIDATES: usize = 2;

pub async fn build_jit_skill_guidance(
    workspace_root: &Path,
    requested_skill_ids: &[String],
    max_chars: usize,
) -> Result<Option<String>> {
    let mut selected = Vec::new();
    let mut dedupe = HashSet::new();
    for id in requested_skill_ids {
        if dedupe.insert(id.clone()) {
            selected.push(id.clone());
        }
        if selected.len() >= MAX_SKILL_CANDIDATES {
            break;
        }
    }
    if selected.is_empty() {
        return Ok(None);
    }

    let mut sections = Vec::new();
    let mut used = 0usize;
    for id in selected {
        let Some(skill) = load_vault_skill_by_id(workspace_root, &id).await? else {
            continue;
        };
        let priority = match skill.priority {
            SkillPriority::Mandatory => "mandatory",
            SkillPriority::Conditional => "conditional",
        };
        let snippet = format!(
            "Skill: {}\nTitle: {}\nPriority: {}\nProcedure:\n{}",
            skill.id, skill.title, priority, skill.body
        );
        if used + snippet.len() > max_chars {
            break;
        }
        used += snippet.len();
        sections.push(snippet);
    }
    if sections.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "[JIT SKILL GUIDANCE]\nApply only relevant skills below. Keep tool args fully schema-valid.\n{}\n[/JIT SKILL GUIDANCE]",
        sections.join("\n\n")
    )))
}
