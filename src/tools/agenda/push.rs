use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;

use super::AgendaTask;
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct AgendaPushArgs {
    pub description: String,
}

pub struct AgendaPushTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for AgendaPushTool {
    fn name(&self) -> &'static str {
        "agenda:push"
    }
    fn description(&self) -> &'static str {
        "Queue a background task for later execution."
    }
    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(AgendaPushArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: AgendaPushArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;
        if args.description.len() > 200 {
            return Err(FcpError::SchemaViolation(
                "Description must be <= 200 chars".to_string(),
            ));
        }

        let agenda_path = crate::vault_layout::agenda_json(&self.workspace_root);
        let mut tasks: Vec<AgendaTask> = Vec::new();

        if agenda_path.exists() {
            let content = fs::read_to_string(&agenda_path)
                .await
                .map_err(FcpError::Io)?;
            if !content.trim().is_empty() {
                tasks = serde_json::from_str(&content).map_err(FcpError::ParseFault)?;
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let id = super::new_task_id();

        tasks.push(AgendaTask {
            id: id.clone(),
            created_at: timestamp,
            description: args.description,
            status: "pending".to_string(),
            alarm_id: None,
            kind: super::AgendaTaskKind::User,
            plan: None,
        });

        let new_content =
            serde_json::to_string_pretty(&tasks).map_err(|e| FcpError::Config(e.to_string()))?;
        fs::create_dir_all(crate::vault_layout::tools_dir(&self.workspace_root))
            .await
            .map_err(FcpError::Io)?;
        fs::write(&agenda_path, new_content)
            .await
            .map_err(FcpError::Io)?;

        Ok(format!(
            "SUCCESS: Task [{}] queued for background execution.",
            id
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test(flavor = "current_thread")]
    async fn test_agenda_push_creates_file() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = AgendaPushTool {
            workspace_root: dir.path().to_path_buf(),
        };
        let args = serde_json::json!({ "description": "Test task" });

        let result = tool.execute(args).await?;
        assert!(result.starts_with("SUCCESS: Task ["));

        let content = fs::read_to_string(crate::vault_layout::agenda_json(dir.path()))
            .await
            .unwrap();
        let tasks: Vec<AgendaTask> = serde_json::from_str(&content).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].description, "Test task");
        Ok(())
    }
}
