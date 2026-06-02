//! Canonical paths under a **vault root**: the directory the operator `cd`s into before `eris chat`
//! (same as [`crate::config::AppConfig::active_vault`]), not `vault_root` + `workspace` from TOML.

use std::path::{Path, PathBuf};

pub fn fcp_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".fcp")
}

/// Main TOML config (Figment merge target: `.fcp/config.toml` relative to cwd at load time).
pub fn config_toml(workspace_root: &Path) -> PathBuf {
    fcp_dir(workspace_root).join("config.toml")
}

/// Text seal (`agent=`, `model=`, `sealed_at=`) — same shape as ignition.
pub fn seal(workspace_root: &Path) -> PathBuf {
    fcp_dir(workspace_root).join("seal")
}

pub fn tools_dir(workspace_root: &Path) -> PathBuf {
    fcp_dir(workspace_root).join("tools")
}

pub fn agenda_json(workspace_root: &Path) -> PathBuf {
    tools_dir(workspace_root).join("agenda.json")
}

pub fn alarms_json(workspace_root: &Path) -> PathBuf {
    tools_dir(workspace_root).join("alarms.json")
}

/// Cached `vault:taglist` snapshot built once at chat startup and rebuilt lazily after
/// successful `vault:write` calls under `30_Synthesis/`.
pub fn taglist_json(workspace_root: &Path) -> PathBuf {
    tools_dir(workspace_root).join("taglist.json")
}

pub fn telemetry_logs_dir(workspace_root: &Path) -> PathBuf {
    fcp_dir(workspace_root).join("telemetry").join("logs")
}

pub fn ephemeral_bin(workspace_root: &Path, workspace: &str) -> PathBuf {
    fcp_dir(workspace_root).join(format!("ephemeral_{}.bin", workspace))
}

/// Web mission cache root (`20_Discourse/web/missions/{mission_id}/`). Not Qdrant-indexed.
pub fn web_missions_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("20_Discourse/web/missions")
}

pub fn web_mission_dir(workspace_root: &Path, mission_id: &str) -> PathBuf {
    web_missions_dir(workspace_root).join(mission_id)
}

pub fn web_mission_manifest(workspace_root: &Path, mission_id: &str) -> PathBuf {
    web_mission_dir(workspace_root, mission_id).join("manifest.json")
}

pub fn web_mission_prose(workspace_root: &Path, mission_id: &str) -> PathBuf {
    web_mission_dir(workspace_root, mission_id).join("mission.md")
}

pub fn web_page_dir(workspace_root: &Path, mission_id: &str, artifact_id: &str) -> PathBuf {
    web_mission_dir(workspace_root, mission_id)
        .join("pages")
        .join(artifact_id)
}
