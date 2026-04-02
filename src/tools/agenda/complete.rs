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
pub struct AgendaCompleteArgs {
    pub task_id: String,
    pub result_summary: String,
}

pub struct AgendaCompleteTool {
    pub workspace_root: PathBuf,
    pub reschedule_tx: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl Tool for AgendaCompleteTool {
    fn name(&self) -> &'static str { "agenda:complete" }
    fn description(&self) -> &'static str { "Mark a background task as complete and log the result." }
    fn parameters_schema(&self) -> schemars::schema::RootSchema { schemars::schema_for!(AgendaCompleteArgs) }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: AgendaCompleteArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        let agenda_path = crate::vault_layout::agenda_json(&self.workspace_root);
        
        if !agenda_path.exists() {
            return Err(FcpError::ToolFault { tool_name: self.name().into(), reason: "Agenda file not found".into() });
        }

        let content = fs::read_to_string(&agenda_path).await.map_err(FcpError::Io)?;
        let mut tasks: Vec<AgendaTask> = serde_json::from_str(&content).map_err(FcpError::ParseFault)?;

        let initial_len = tasks.len();
        let removed = tasks.iter().find(|t| t.id == args.task_id).cloned();
        tasks.retain(|t| t.id != args.task_id);

        if tasks.len() == initial_len {
            return Err(FcpError::ToolFault { tool_name: self.name().into(), reason: format!("Task ID {} not found", args.task_id) });
        }

        if let Some(t) = removed {
            if let Some(aid) = t.alarm_id {
                let alarm_path = crate::vault_layout::alarms_json(&self.workspace_root);
                if remove_alarm_by_id(&alarm_path, &aid).await? {
                    let _ = self.reschedule_tx.send(());
                }
            }
        }

        let new_content = serde_json::to_string_pretty(&tasks).map_err(|e| FcpError::Config(e.to_string()))?;
        fs::create_dir_all(crate::vault_layout::tools_dir(&self.workspace_root))
            .await
            .map_err(FcpError::Io)?;
        fs::write(&agenda_path, new_content).await.map_err(FcpError::Io)?;

        let episodic_dir = self.workspace_root.join("10_Episodic");
        if !episodic_dir.exists() {
            fs::create_dir_all(&episodic_dir).await.map_err(FcpError::Io)?;
        }

        let log_path = episodic_dir.join("Tasks.md");
        let mut log_content = String::new();
        if log_path.exists() {
            log_content = fs::read_to_string(&log_path).await.map_err(FcpError::Io)?;
        }
        
        log_content.push_str(&format!("\n## Task [{}]\n**Result:** {}\n", args.task_id, args.result_summary));
        fs::write(&log_path, log_content).await.map_err(FcpError::Io)?;

        Ok(format!("SUCCESS: Task [{}] marked complete and logged.", args.task_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_agenda_complete_missing_task() -> Result<()> {
        let dir = tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let tool = AgendaCompleteTool {
            workspace_root: dir.path().to_path_buf(),
            reschedule_tx: tx,
        };
        let result = tool.execute(serde_json::json!({ "task_id": "1234", "result_summary": "done" })).await;
        assert!(result.is_err());
        Ok(())
    }
}
