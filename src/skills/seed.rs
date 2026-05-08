use std::path::Path;

use crate::executive::error::{FcpError, Result};

use super::defaults::embedded_defaults;
use super::paths::runtime_skills_dir;
use super::types::SkillSeedReport;

pub async fn seed_runtime_skills(workspace_root: &Path) -> Result<SkillSeedReport> {
    let mut report = SkillSeedReport::default();
    let skills_dir = runtime_skills_dir(workspace_root);
    tokio::fs::create_dir_all(&skills_dir).await?;
    for skill in embedded_defaults() {
        let path = skills_dir.join(skill.file_name);
        match tokio::fs::metadata(&path).await {
            Ok(_) => {
                report.skipped_existing += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::write(&path, skill.raw).await?;
                report.copied += 1;
            }
            Err(e) => {
                return Err(FcpError::Io(e));
            }
        }
    }
    Ok(report)
}
