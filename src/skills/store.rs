use std::path::Path;

use crate::executive::error::{FcpError, Result};

use super::parse::parse_skill_markdown;
use super::paths::runtime_skills_dir;
use super::types::{SkillDoc, SkillPriority};

pub async fn list_vault_skills(workspace_root: &Path) -> Result<Vec<SkillDoc>> {
    let dir = runtime_skills_dir(workspace_root);
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(FcpError::Io(e)),
    };
    while let Some(entry) = entries.next_entry().await.map_err(FcpError::Io)? {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let raw = tokio::fs::read_to_string(&path).await.map_err(FcpError::Io)?;
        let parsed = parse_skill_markdown(&raw)?;
        out.push(parsed);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

pub async fn load_vault_skill_by_id(workspace_root: &Path, skill_id: &str) -> Result<Option<SkillDoc>> {
    let skills = list_vault_skills(workspace_root).await?;
    Ok(skills.into_iter().find(|s| s.id == skill_id))
}

#[derive(Debug, Clone)]
pub struct SkillCreateInput {
    pub id: String,
    pub title: String,
    pub priority: SkillPriority,
    pub triggers: Vec<String>,
    pub body: String,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct SkillWriteReceipt {
    pub relative_path: String,
    pub overwritten: bool,
    pub skill: SkillDoc,
}

pub async fn create_or_update_vault_skill(
    workspace_root: &Path,
    input: SkillCreateInput,
) -> Result<SkillWriteReceipt> {
    validate_skill_id(&input.id)?;
    if input.title.trim().is_empty() {
        return Err(FcpError::SchemaViolation("title cannot be empty".to_string()));
    }
    if input.body.trim().is_empty() {
        return Err(FcpError::SchemaViolation("body cannot be empty".to_string()));
    }
    if input.triggers.is_empty() {
        return Err(FcpError::SchemaViolation(
            "triggers cannot be empty".to_string(),
        ));
    }
    if input.triggers.iter().any(|t| t.trim().is_empty()) {
        return Err(FcpError::SchemaViolation(
            "triggers must not contain empty entries".to_string(),
        ));
    }

    let relative_path = format!("10_Topology/skills/{}.md", input.id);
    let target = runtime_skills_dir(workspace_root).join(format!("{}.md", input.id));
    if !target.starts_with(workspace_root) {
        return Err(FcpError::ToolFault {
            tool_name: "skills:create".to_string(),
            reason: "Path Traversal Denied".to_string(),
        });
    }

    let existed = tokio::fs::metadata(&target).await.is_ok();
    if existed && !input.overwrite {
        return Err(FcpError::ToolFault {
            tool_name: "skills:create".to_string(),
            reason: format!("Skill already exists: {} (set overwrite=true to replace)", input.id),
        });
    }

    tokio::fs::create_dir_all(runtime_skills_dir(workspace_root))
        .await
        .map_err(FcpError::Io)?;

    let raw = render_skill_markdown(&input);
    // Parse before write so malformed construction fails fast.
    let parsed = parse_skill_markdown(&raw)?;
    if parsed.id != input.id {
        return Err(FcpError::Config(
            "Rendered skill id does not match requested id".to_string(),
        ));
    }

    tokio::fs::write(&target, raw).await.map_err(FcpError::Io)?;
    // Read+parse back for round-trip validation.
    let read_back = tokio::fs::read_to_string(&target).await.map_err(FcpError::Io)?;
    let parsed_back = parse_skill_markdown(&read_back)?;
    Ok(SkillWriteReceipt {
        relative_path,
        overwritten: existed,
        skill: parsed_back,
    })
}

fn render_skill_markdown(input: &SkillCreateInput) -> String {
    let priority = match input.priority {
        SkillPriority::Mandatory => "mandatory",
        SkillPriority::Conditional => "conditional",
    };
    let triggers = input
        .triggers
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "---\nid: {}\ntitle: {}\npriority: {}\ntriggers: {}\n---\n{}",
        input.id.trim(),
        input.title.trim(),
        priority,
        triggers,
        input.body.trim()
    )
}

fn validate_skill_id(id: &str) -> Result<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(FcpError::SchemaViolation("id cannot be empty".to_string()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(FcpError::SchemaViolation(
            "id must be kebab-case ascii (a-z, 0-9, -)".to_string(),
        ));
    }
    if trimmed.starts_with('-') || trimmed.ends_with('-') || trimmed.contains("--") {
        return Err(FcpError::SchemaViolation(
            "id has invalid dash placement".to_string(),
        ));
    }
    Ok(())
}
