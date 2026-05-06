use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};
use crate::tools::specs::DESCRIPTOR_TOMLS;

#[derive(Debug, Clone, Deserialize)]
pub struct ToolExample {
    pub name: String,
    #[serde(default)]
    pub args: Value,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDescriptor {
    pub descriptor_version: u32,
    pub tool_name: String,
    pub short_description: String,
    pub when_to_use: Option<String>,
    pub when_not_to_use: Option<String>,
    #[serde(default)]
    pub routing_hints: Vec<String>,
    #[serde(default)]
    pub examples_good: Vec<ToolExample>,
    #[serde(default)]
    pub examples_bad: Vec<ToolExample>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolDescriptorRegistry {
    by_tool: HashMap<String, ToolDescriptor>,
}

impl ToolDescriptorRegistry {
    pub fn load_embedded() -> Result<Self> {
        let mut by_tool = HashMap::new();

        for raw in DESCRIPTOR_TOMLS {
            let descriptor: ToolDescriptor = toml::from_str(raw).map_err(|e| {
                FcpError::Config(format!("Failed parsing embedded descriptor: {}", e))
            })?;
            Self::validate_descriptor(&descriptor)?;
            let tool_name = descriptor.tool_name.clone();
            if by_tool.insert(tool_name.clone(), descriptor).is_some() {
                return Err(FcpError::Config(format!(
                    "Duplicate embedded descriptor for tool_name: {}",
                    tool_name
                )));
            }
        }

        Ok(Self { by_tool })
    }

    fn validate_descriptor(desc: &ToolDescriptor) -> Result<()> {
        if desc.descriptor_version == 0 {
            return Err(FcpError::Config(
                "descriptor_version must be >= 1".to_string(),
            ));
        }
        if desc.tool_name.trim().is_empty() || desc.short_description.trim().is_empty() {
            return Err(FcpError::Config(
                "tool_name and short_description are required".to_string(),
            ));
        }
        Ok(())
    }

    pub fn get(&self, tool_name: &str) -> Option<&ToolDescriptor> {
        self.by_tool.get(tool_name)
    }

    pub fn len(&self) -> usize {
        self.by_tool.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_tool.is_empty()
    }

    pub fn assert_covers_registered_tools(&self, registered_tools: &[String]) -> Result<()> {
        let registered_set: HashSet<String> = registered_tools.iter().cloned().collect();
        let descriptor_set: HashSet<String> = self.by_tool.keys().cloned().collect();

        let missing = registered_set
            .difference(&descriptor_set)
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(FcpError::Config(format!(
                "Descriptor coverage mismatch. missing={:?}",
                missing
            )));
        }
        let orphan = descriptor_set
            .difference(&registered_set)
            .cloned()
            .collect::<Vec<_>>();
        if !orphan.is_empty() {
            tracing::warn!(orphan = ?orphan, "Orphan descriptors present for currently unregistered tools");
        }
        Ok(())
    }
}
