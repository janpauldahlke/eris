use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::executive::error::{FcpError, Result};

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
    pub async fn load_from_dir(dir: &Path) -> Result<Self> {
        let mut entries = tokio::fs::read_dir(dir).await.map_err(FcpError::Io)?;
        let mut by_tool = HashMap::new();
        let mut seen_files = HashSet::new();

        while let Some(entry) = entries.next_entry().await.map_err(FcpError::Io)? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| FcpError::Config(format!("Invalid descriptor filename: {}", path.display())))?
                .to_string();
            if !seen_files.insert(file_name.clone()) {
                return Err(FcpError::Config(format!("Duplicate descriptor filename: {file_name}")));
            }

            let raw = tokio::fs::read_to_string(&path).await.map_err(FcpError::Io)?;
            let descriptor: ToolDescriptor = toml::from_str(&raw)
                .map_err(|e| FcpError::Config(format!("Failed parsing {}: {}", path.display(), e)))?;
            Self::validate_descriptor(&descriptor, &path)?;
            if by_tool
                .insert(descriptor.tool_name.clone(), descriptor)
                .is_some()
            {
                return Err(FcpError::Config(format!(
                    "Duplicate tool_name in descriptors: {}",
                    path.display()
                )));
            }
        }

        Ok(Self { by_tool })
    }

    fn validate_descriptor(desc: &ToolDescriptor, path: &Path) -> Result<()> {
        if desc.descriptor_version == 0 {
            return Err(FcpError::Config(format!(
                "descriptor_version must be >= 1 in {}",
                path.display()
            )));
        }
        if desc.tool_name.trim().is_empty() || desc.short_description.trim().is_empty() {
            return Err(FcpError::Config(format!(
                "tool_name and short_description are required in {}",
                path.display()
            )));
        }
        Ok(())
    }

    pub fn get(&self, tool_name: &str) -> Option<&ToolDescriptor> {
        self.by_tool.get(tool_name)
    }

    pub fn len(&self) -> usize {
        self.by_tool.len()
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

