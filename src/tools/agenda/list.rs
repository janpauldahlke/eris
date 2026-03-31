use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::fs;

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;
use super::AgendaTask;

#[derive(Deserialize, JsonSchema)]
pub struct AgendaListArgs {}

pub struct AgendaListTool {
    pub workspace_root: PathBuf,
}

#[async_trait]
impl Tool for AgendaListTool {
    fn name(&self) -> &'static str { "agenda:list" }
    fn description(&self) -> &'static str { "List the top 5 pending background tasks." }
    fn parameters_schema(&self) -> schemars::schema::RootSchema { schemars::schema_for!(AgendaListArgs) }

    async fn execute(&self, _args: Value) -> Result<String> {
        let agenda_path = self.workspace_root.join(".fcp_agenda.json");
        if !agenda_path.exists() {
            return Ok("No pending tasks.".to_string());
        }

        let content = fs::read_to_string(&agenda_path).await.map_err(FcpError::Io)?;
        if content.trim().is_empty() {
            return Ok("No pending tasks.".to_string());
        }

        let tasks: Vec<AgendaTask> = serde_json::from_str(&content).map_err(|e| FcpError::ParseFault(e))?;
        if tasks.is_empty() {
            return Ok("No pending tasks.".to_string());
        }

        let mut output = String::new();
        for (i, task) in tasks.iter().take(5).enumerate() {
            output.push_str(&format!("{}. [{}] - {}\n", i + 1, task.id, task.description));
        }

        Ok(output.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_agenda_list_empty() -> Result<()> {
        let dir = tempdir().unwrap();
        let tool = AgendaListTool { workspace_root: dir.path().to_path_buf() };
        let result = tool.execute(serde_json::json!({})).await?;
        assert_eq!(result, "No pending tasks.");
        Ok(())
    }
}
