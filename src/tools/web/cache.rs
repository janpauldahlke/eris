//! Vault-backed web mission cache under `20_Discourse/web/missions/{mission_id}/`.

use crate::executive::error::{FcpError, Result};
use crate::tools::web::artifact::WebOutboundLink;
use crate::vault_layout;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Active,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchBudgetRecord {
    pub max: u32,
    pub used: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedPageRecord {
    pub url: String,
    pub artifact_id: String,
    pub normalized_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebMissionManifest {
    pub mission_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_note: Option<String>,
    pub status: MissionStatus,
    pub fetch_budget: FetchBudgetRecord,
    #[serde(default)]
    pub fetched: Vec<FetchedPageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPageRecord {
    pub url: String,
    pub normalized_url: String,
    pub fetched_at: DateTime<Utc>,
    pub host: String,
    pub truncated: bool,
    pub chunk_count: u32,
}

pub struct WebMissionStore {
    vault_root: PathBuf,
}

impl WebMissionStore {
    pub fn new(vault_root: impl AsRef<Path>) -> Self {
        Self {
            vault_root: vault_root.as_ref().to_path_buf(),
        }
    }

    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    pub fn ensure_missions_root(&self) -> Result<()> {
        fs::create_dir_all(vault_layout::web_missions_dir(&self.vault_root)).map_err(FcpError::Io)
    }

    /// Delete every mission directory under [`vault_layout::web_missions_dir`].
    ///
    /// Used on chat exit so fetched page chunks do not accumulate in `20_Discourse/web/`.
    /// Returns the number of mission directories removed.
    pub fn purge_all_missions(&self) -> Result<u32> {
        let root = vault_layout::web_missions_dir(&self.vault_root);
        if !root.is_dir() {
            return Ok(0);
        }
        let mut removed = 0u32;
        for entry in fs::read_dir(&root).map_err(FcpError::Io)? {
            let entry = entry.map_err(FcpError::Io)?;
            let path = entry.path();
            if entry.file_type().map_err(FcpError::Io)?.is_dir() {
                fs::remove_dir_all(&path).map_err(FcpError::Io)?;
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }

    pub fn create_mission(
        &self,
        mission_id: &str,
        mission_note: Option<&str>,
        budget_max: u32,
    ) -> Result<WebMissionManifest> {
        validate_path_token(mission_id, "mission_id")?;
        self.ensure_missions_root()?;

        let mission_dir = vault_layout::web_mission_dir(&self.vault_root, mission_id);
        if mission_dir.exists() {
            return self.load_manifest(mission_id);
        }

        fs::create_dir_all(mission_dir.join("pages")).map_err(FcpError::Io)?;

        let manifest = WebMissionManifest {
            mission_id: mission_id.to_string(),
            mission_note: mission_note.map(str::to_string),
            status: MissionStatus::Active,
            fetch_budget: FetchBudgetRecord {
                max: budget_max.max(1),
                used: 0,
            },
            fetched: Vec::new(),
            stop_reason: None,
        };
        self.save_manifest(&manifest)?;
        if let Some(note) = mission_note.filter(|n| !n.trim().is_empty()) {
            self.write_mission_prose(mission_id, note)?;
        }
        Ok(manifest)
    }

    pub fn load_manifest(&self, mission_id: &str) -> Result<WebMissionManifest> {
        validate_path_token(mission_id, "mission_id")?;
        let path = vault_layout::web_mission_manifest(&self.vault_root, mission_id);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FcpError::SchemaViolation(format!("web: mission not found: {mission_id}"))
            } else {
                FcpError::Io(e)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(FcpError::ParseFault)
    }

    pub fn save_manifest(&self, manifest: &WebMissionManifest) -> Result<()> {
        validate_path_token(&manifest.mission_id, "mission_id")?;
        let path = vault_layout::web_mission_manifest(&self.vault_root, &manifest.mission_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(FcpError::Io)?;
        }
        let bytes = serde_json::to_vec_pretty(manifest).map_err(FcpError::ParseFault)?;
        fs::write(path, bytes).map_err(FcpError::Io)
    }

    pub fn finalize_mission(
        &self,
        mission_id: &str,
        stop_reason: Option<&str>,
    ) -> Result<WebMissionManifest> {
        let mut manifest = self.load_manifest(mission_id)?;
        manifest.status = MissionStatus::Done;
        if let Some(reason) = stop_reason.filter(|r| !r.trim().is_empty()) {
            manifest.stop_reason = Some(reason.to_string());
        }
        self.save_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn record_page_fetch(
        &self,
        mission_id: &str,
        url: &str,
        normalized_url: &str,
        artifact_id: &str,
    ) -> Result<WebMissionManifest> {
        let mut manifest = self.load_manifest(mission_id)?;
        manifest.record_page_fetch(url, normalized_url, artifact_id);
        self.save_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn write_page(
        &self,
        mission_id: &str,
        artifact_id: &str,
        page: &WebPageRecord,
        chunks: &[String],
        links: &[WebOutboundLink],
    ) -> Result<()> {
        validate_path_token(mission_id, "mission_id")?;
        validate_path_token(artifact_id, "artifact_id")?;

        let page_dir = vault_layout::web_page_dir(&self.vault_root, mission_id, artifact_id);
        let chunks_dir = page_dir.join("chunks");
        fs::create_dir_all(&chunks_dir).map_err(FcpError::Io)?;

        let page_meta = WebPageRecord {
            chunk_count: chunks.len().min(u32::MAX as usize) as u32,
            ..page.clone()
        };
        let page_json = serde_json::to_vec_pretty(&page_meta).map_err(FcpError::ParseFault)?;
        fs::write(page_dir.join("page.json"), page_json).map_err(FcpError::Io)?;

        let links_json = serde_json::to_vec_pretty(links).map_err(FcpError::ParseFault)?;
        fs::write(page_dir.join("links.json"), links_json).map_err(FcpError::Io)?;

        for (idx, chunk) in chunks.iter().enumerate() {
            let name = chunk_filename(idx);
            fs::write(chunks_dir.join(name), chunk.as_bytes()).map_err(FcpError::Io)?;
        }
        Ok(())
    }

    pub fn read_page_meta(&self, mission_id: &str, artifact_id: &str) -> Result<WebPageRecord> {
        validate_path_token(mission_id, "mission_id")?;
        validate_path_token(artifact_id, "artifact_id")?;
        let path = vault_layout::web_page_dir(&self.vault_root, mission_id, artifact_id)
            .join("page.json");
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FcpError::SchemaViolation(format!(
                    "web: page not found mission={mission_id} artifact={artifact_id}"
                ))
            } else {
                FcpError::Io(e)
            }
        })?;
        serde_json::from_slice(&bytes).map_err(FcpError::ParseFault)
    }

    pub fn list_chunk_indices(&self, mission_id: &str, artifact_id: &str) -> Result<Vec<u32>> {
        validate_path_token(mission_id, "mission_id")?;
        validate_path_token(artifact_id, "artifact_id")?;
        let chunks_dir =
            vault_layout::web_page_dir(&self.vault_root, mission_id, artifact_id).join("chunks");
        if !chunks_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut indices = Vec::new();
        for entry in fs::read_dir(&chunks_dir).map_err(FcpError::Io)? {
            let entry = entry.map_err(FcpError::Io)?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".md") {
                if let Ok(idx) = stem.parse::<u32>() {
                    indices.push(idx);
                }
            }
        }
        indices.sort_unstable();
        Ok(indices)
    }

    pub fn read_chunk(
        &self,
        mission_id: &str,
        artifact_id: &str,
        index: u32,
    ) -> Result<String> {
        validate_path_token(mission_id, "mission_id")?;
        validate_path_token(artifact_id, "artifact_id")?;
        let path = vault_layout::web_page_dir(&self.vault_root, mission_id, artifact_id)
            .join("chunks")
            .join(chunk_filename(index as usize));
        fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FcpError::SchemaViolation(format!(
                    "web: chunk {index:03} not found for artifact {artifact_id}"
                ))
            } else {
                FcpError::Io(e)
            }
        })
    }

    pub fn read_links(&self, mission_id: &str, artifact_id: &str) -> Result<Vec<WebOutboundLink>> {
        validate_path_token(mission_id, "mission_id")?;
        validate_path_token(artifact_id, "artifact_id")?;
        let path = vault_layout::web_page_dir(&self.vault_root, mission_id, artifact_id)
            .join("links.json");
        let bytes = fs::read(&path).map_err(FcpError::Io)?;
        serde_json::from_slice(&bytes).map_err(FcpError::ParseFault)
    }

    fn write_mission_prose(&self, mission_id: &str, mission_note: &str) -> Result<()> {
        let path = vault_layout::web_mission_prose(&self.vault_root, mission_id);
        let body = format!(
            "# Mission\n\n{mission_note}\n\n## Checklist\n\n- [ ] Review fetched pages with `web:find`\n- [ ] Answer the user or fetch more pages within budget\n"
        );
        fs::write(path, body).map_err(FcpError::Io)
    }
}

impl WebMissionManifest {
    pub fn budget_remaining(&self) -> u32 {
        self.fetch_budget.max.saturating_sub(self.fetch_budget.used)
    }

    pub fn record_page_fetch(&mut self, url: &str, normalized_url: &str, artifact_id: &str) {
        self.fetched.push(FetchedPageRecord {
            url: url.to_string(),
            artifact_id: artifact_id.to_string(),
            normalized_url: normalized_url.to_string(),
        });
        self.fetch_budget.used = self.fetch_budget.used.saturating_add(1);
        if self.fetch_budget.used >= self.fetch_budget.max {
            self.stop_reason = Some("budget_exhausted".to_string());
        }
    }
}

fn chunk_filename(index: usize) -> String {
    format!("{index:03}.md")
}

fn validate_path_token(id: &str, label: &str) -> Result<()> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(FcpError::SchemaViolation(format!("web: empty {label}")));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(FcpError::SchemaViolation(format!(
            "web: invalid {label} (path characters forbidden)"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(FcpError::SchemaViolation(format!(
            "web: invalid {label} (use UUID or alphanumeric id)"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::web::ledger::{host_from_normalized_url, normalize_url};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn create_mission_writes_manifest_and_prose() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WebMissionStore::new(dir.path());
        let mid = Uuid::new_v4().to_string();
        let manifest = store
            .create_mission(&mid, Some("Find product X price"), 3)
            .expect("create");
        assert_eq!(manifest.mission_id, mid);
        assert_eq!(manifest.fetch_budget.max, 3);
        assert_eq!(manifest.status, MissionStatus::Active);

        let loaded = store.load_manifest(&mid).expect("load");
        assert_eq!(loaded.mission_note.as_deref(), Some("Find product X price"));

        let prose = fs::read_to_string(vault_layout::web_mission_prose(dir.path(), &mid))
            .expect("prose");
        assert!(prose.contains("Find product X price"));
        assert!(prose.contains("web:find"));
    }

    #[test]
    fn write_page_and_read_chunks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WebMissionStore::new(dir.path());
        let mid = Uuid::new_v4().to_string();
        let aid = Uuid::new_v4().to_string();
        store.create_mission(&mid, None, 2).expect("create");

        let url = "https://example.com/item?q=1";
        let norm = normalize_url(url).expect("norm");
        let host = host_from_normalized_url(&norm).expect("host");
        store
            .write_page(
                &mid,
                &aid,
                &WebPageRecord {
                    url: url.to_string(),
                    normalized_url: norm.clone(),
                    fetched_at: Utc::now(),
                    host,
                    truncated: false,
                    chunk_count: 2,
                },
                &["chunk zero".into(), "chunk one".into()],
                &[WebOutboundLink {
                    url: "https://example.com/next".into(),
                    anchor_text: Some("next".into()),
                    rank: 1,
                }],
            )
            .expect("write");

        let meta = store.read_page_meta(&mid, &aid).expect("meta");
        assert_eq!(meta.chunk_count, 2);
        assert_eq!(store.read_chunk(&mid, &aid, 0).expect("c0"), "chunk zero");
        assert_eq!(store.read_chunk(&mid, &aid, 1).expect("c1"), "chunk one");
        let links = store.read_links(&mid, &aid).expect("links");
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn record_page_fetch_updates_manifest_budget() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WebMissionStore::new(dir.path());
        let mid = Uuid::new_v4().to_string();
        store.create_mission(&mid, None, 1).expect("create");
        let aid = Uuid::new_v4().to_string();
        let manifest = store
            .record_page_fetch(&mid, "https://a.test/", "https://a.test/", &aid)
            .expect("record");
        assert_eq!(manifest.fetch_budget.used, 1);
        assert_eq!(
            manifest.stop_reason.as_deref(),
            Some("budget_exhausted")
        );
        assert_eq!(manifest.budget_remaining(), 0);
    }

    #[test]
    fn finalize_mission_sets_done() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WebMissionStore::new(dir.path());
        let mid = Uuid::new_v4().to_string();
        store.create_mission(&mid, None, 2).expect("create");
        let done = store
            .finalize_mission(&mid, Some("user_ended"))
            .expect("finalize");
        assert_eq!(done.status, MissionStatus::Done);
        assert_eq!(done.stop_reason.as_deref(), Some("user_ended"));
    }

    #[test]
    fn purge_all_missions_removes_every_mission_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WebMissionStore::new(dir.path());
        let mid_a = Uuid::new_v4().to_string();
        let mid_b = Uuid::new_v4().to_string();
        store.create_mission(&mid_a, None, 1).expect("a");
        store.create_mission(&mid_b, None, 1).expect("b");
        let removed = store.purge_all_missions().expect("purge");
        assert_eq!(removed, 2);
        assert!(vault_layout::web_missions_dir(dir.path()).is_dir());
        assert!(
            fs::read_dir(vault_layout::web_missions_dir(dir.path()))
                .expect("read")
                .next()
                .is_none()
        );
    }

    #[test]
    fn rejects_path_traversal_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WebMissionStore::new(dir.path());
        let err = store.create_mission("../evil", None, 1).unwrap_err();
        assert!(matches!(err, FcpError::SchemaViolation(_)));
    }
}
