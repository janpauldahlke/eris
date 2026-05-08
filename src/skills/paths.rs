use std::path::{Path, PathBuf};

pub fn runtime_skills_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("10_Topology").join("skills")
}
