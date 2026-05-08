use crate::executive::error::{FcpError, Result};

use super::types::{SkillDoc, SkillPriority};

pub fn parse_skill_markdown(raw: &str) -> Result<SkillDoc> {
    let mut lines = raw.lines();
    if lines.next() != Some("---") {
        return Err(FcpError::Config(
            "Skill markdown missing frontmatter start '---'".to_string(),
        ));
    }

    let mut id = None::<String>;
    let mut title = None::<String>;
    let mut priority = None::<SkillPriority>;
    let mut triggers = Vec::<String>::new();

    let mut in_frontmatter = true;
    let mut body_lines = Vec::<String>::new();
    for line in raw.lines().skip(1) {
        if in_frontmatter {
            if line.trim() == "---" {
                in_frontmatter = false;
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim();
                let value = v.trim();
                match key {
                    "id" => id = Some(value.to_string()),
                    "title" => title = Some(value.to_string()),
                    "priority" => {
                        priority = Some(match value {
                            "mandatory" => SkillPriority::Mandatory,
                            "conditional" => SkillPriority::Conditional,
                            other => {
                                return Err(FcpError::Config(format!(
                                    "Invalid skill priority: {}",
                                    other
                                )));
                            }
                        });
                    }
                    "triggers" => {
                        triggers = value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                            .collect();
                    }
                    _ => {}
                }
            }
        } else {
            body_lines.push(line.to_string());
        }
    }

    let id = id.ok_or_else(|| FcpError::Config("Skill frontmatter missing id".to_string()))?;
    let title =
        title.ok_or_else(|| FcpError::Config("Skill frontmatter missing title".to_string()))?;
    let priority = priority
        .ok_or_else(|| FcpError::Config("Skill frontmatter missing priority".to_string()))?;
    if triggers.is_empty() {
        return Err(FcpError::Config(
            "Skill frontmatter missing triggers".to_string(),
        ));
    }
    let body = body_lines.join("\n").trim().to_string();
    if body.is_empty() {
        return Err(FcpError::Config("Skill body is empty".to_string()));
    }
    Ok(SkillDoc {
        id,
        title,
        priority,
        triggers,
        body,
    })
}
