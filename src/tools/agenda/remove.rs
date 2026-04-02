use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;
use tokio::sync::mpsc;

use crate::executive::error::{FcpError, Result};
use crate::tools::clock::remove_alarm_by_id;
use crate::tools::traits::Tool;
use super::AgendaTask;

#[derive(Deserialize, JsonSchema)]
pub struct AgendaRemoveArgs {
    /// Exact task id from agenda:list (e.g. four hex chars).
    pub task_id: Option<String>,
    /// Substring to match against task descriptions (case-insensitive). If zero tasks match, fails with
    /// a hint to use agenda:list. If two or more match, fails without removing anything and lists the
    /// conflicting ids/descriptions so the caller can retry with `task_id` or a narrower substring.
    pub description_match: Option<String>,
}

pub struct AgendaRemoveTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for AgendaRemoveTool {
    fn name(&self) -> &'static str {
        "agenda:remove"
    }
    fn description(&self) -> &'static str {
        "Remove a pending agenda task by id or by a unique description substring (cancellation; does not log to episodic Tasks.md)."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(AgendaRemoveArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: AgendaRemoveArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let tid = args
            .task_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let dm = args
            .description_match
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        match (&tid, &dm) {
            (Some(_), Some(_)) => {
                return Err(FcpError::SchemaViolation(
                    "Provide exactly one of task_id or description_match, not both.".to_string(),
                ));
            }
            (None, None) => {
                return Err(FcpError::SchemaViolation(
                    "Provide exactly one of task_id or description_match.".to_string(),
                ));
            }
            _ => {}
        }

        if let Some(ref m) = dm {
            if m.len() > 200 {
                return Err(FcpError::SchemaViolation(
                    "description_match must be <= 200 chars".to_string(),
                ));
            }
        }

        let agenda_path = crate::vault_layout::agenda_json(&self.workspace_root);

        if !agenda_path.exists() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Agenda file not found".into(),
            });
        }

        let content = fs::read_to_string(&agenda_path).await.map_err(FcpError::Io)?;
        let mut tasks: Vec<AgendaTask> = serde_json::from_str(&content).map_err(FcpError::ParseFault)?;

        if let Some(id) = tid {
            let initial_len = tasks.len();
            let victim = tasks.iter().find(|t| t.id == id).cloned();
            tasks.retain(|t| t.id != id);
            if tasks.len() == initial_len {
                return Err(FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!("Task ID {} not found", id),
                });
            }
            if let Some(t) = victim {
                if let Some(aid) = t.alarm_id {
                    let alarm_path = crate::vault_layout::alarms_json(&self.workspace_root);
                    if remove_alarm_by_id(&alarm_path, &aid).await? {
                        let _ = self.reschedule_tx.send(());
                    }
                }
            }
            let new_content =
                serde_json::to_string_pretty(&tasks).map_err(|e| FcpError::Config(e.to_string()))?;
            fs::create_dir_all(crate::vault_layout::tools_dir(&self.workspace_root))
                .await
                .map_err(FcpError::Io)?;
            fs::write(&agenda_path, new_content).await.map_err(FcpError::Io)?;
            return Ok(format!("SUCCESS: Task [{}] removed from agenda.", id));
        }

        let m = dm.ok_or_else(|| {
            FcpError::SchemaViolation("Provide exactly one of task_id or description_match.".to_string())
        })?;
        let needle = m.to_lowercase();

        let matches: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.description.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();

        match matches.len() {
            0 => Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "No task description contains that substring; use agenda:list to see pending tasks, then agenda:remove with task_id or a narrower description_match.".into(),
            }),
            1 => {
                let idx = matches[0];
                let removed = tasks.remove(idx);
                if let Some(aid) = removed.alarm_id {
                    let alarm_path = crate::vault_layout::alarms_json(&self.workspace_root);
                    if remove_alarm_by_id(&alarm_path, &aid).await? {
                        let _ = self.reschedule_tx.send(());
                    }
                }
                let new_content =
                    serde_json::to_string_pretty(&tasks).map_err(|e| FcpError::Config(e.to_string()))?;
                fs::create_dir_all(crate::vault_layout::tools_dir(&self.workspace_root))
                    .await
                    .map_err(FcpError::Io)?;
                fs::write(&agenda_path, new_content).await.map_err(FcpError::Io)?;
                Ok(format!(
                    "SUCCESS: Task [{}] removed from agenda (matched description).",
                    removed.id
                ))
            }
            _ => {
                let mut lines = String::new();
                for idx in matches {
                    let t = &tasks[idx];
                    lines.push_str(&format!("- [{}] {}\n", t.id, t.description));
                }
                Err(FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: format!(
                        "Multiple tasks match that substring; use task_id or a longer unique substring:\n{}",
                        lines.trim_end()
                    ),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_tool(dir: &std::path::Path) -> AgendaRemoveTool {
        let (tx, _rx) = mpsc::unbounded_channel();
        AgendaRemoveTool {
            workspace_root: dir.to_path_buf(),
            reschedule_tx: tx,
        }
    }

    async fn write_agenda(dir: &std::path::Path, json: &str) -> Result<()> {
        let path = crate::vault_layout::agenda_json(dir);
        fs::create_dir_all(crate::vault_layout::tools_dir(dir))
            .await
            .map_err(FcpError::Io)?;
        fs::write(&path, json).await.map_err(FcpError::Io)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_by_task_id() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a03e","created_at":1,"description":"Look for fish","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let out = tool
            .execute(serde_json::json!({ "task_id": "a03e" }))
            .await?;
        assert!(out.contains("a03e"));
        let content = fs::read_to_string(crate::vault_layout::agenda_json(dir.path()))
            .await
            .unwrap();
        let tasks: Vec<AgendaTask> = serde_json::from_str(&content).unwrap();
        assert!(tasks.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_by_description_match() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a03e","created_at":1,"description":"Look for Hagbard's goldfish","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let out = tool
            .execute(serde_json::json!({ "description_match": "goldfish" }))
            .await?;
        assert!(out.contains("a03e"));
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_task_id_not_found() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a03e","created_at":1,"description":"x","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let r = tool
            .execute(serde_json::json!({ "task_id": "ffff" }))
            .await;
        assert!(r.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_nl_zero_matches() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a03e","created_at":1,"description":"alpha","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let r = tool
            .execute(serde_json::json!({ "description_match": "beta" }))
            .await;
        assert!(r.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_nl_ambiguous() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a1","created_at":1,"description":"buy milk","status":"pending"},{"id":"a2","created_at":2,"description":"buy milk later","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let r = tool
            .execute(serde_json::json!({ "description_match": "milk" }))
            .await;
        assert!(r.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_both_selectors_rejected() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a03e","created_at":1,"description":"x","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let r = tool
            .execute(serde_json::json!({ "task_id": "a03e", "description_match": "x" }))
            .await;
        assert!(r.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_remove_neither_selector_rejected() -> Result<()> {
        let dir = tempdir().unwrap();
        write_agenda(
            dir.path(),
            r#"[{"id":"a03e","created_at":1,"description":"x","status":"pending"}]"#,
        )
        .await?;
        let tool = make_tool(dir.path());
        let r = tool.execute(serde_json::json!({})).await;
        assert!(r.is_err());
        Ok(())
    }
}
