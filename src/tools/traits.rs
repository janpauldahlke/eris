use async_trait::async_trait;
use schemars::schema::RootSchema;
use serde_json::Value;
use crate::executive::error::Result;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> RootSchema;
    async fn execute(&self, args: Value) -> Result<String>;
}
