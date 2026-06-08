use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use super::taglist_cache::TaglistCache;
use super::taglist_index::{
    TaglistEntry, TaglistSnapshot, build_synthesis_taglist, load_persisted, persist,
};
use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

const DEFAULT_TOP_K: u32 = 50;
const MAX_TOP_K: u32 = 500;

#[derive(Deserialize, JsonSchema, Default)]
pub struct VaultTaglistArgs {
    /// Maximum number of tags to return (default 50, cap 500). Ignored when `tag` is set.
    #[serde(default)]
    pub top_k: Option<u32>,
    /// Drop entries whose count is below this threshold (default 1).
    #[serde(default)]
    pub min_count: Option<u32>,
    /// Case-insensitive prefix filter on tag name (e.g. `"agent"` matches `agent-loop`).
    #[serde(default)]
    pub prefix: Option<String>,
    /// When set, return only this tag's entry with full path list (case-insensitive exact match).
    #[serde(default)]
    pub tag: Option<String>,
    /// When `tag` is unset, also include the path list per entry. Defaults to `false` for compactness.
    #[serde(default)]
    pub include_paths: Option<bool>,
    /// Force a rebuild even if the cache is clean.
    #[serde(default)]
    pub refresh: Option<bool>,
}

pub struct VaultTaglistTool {
    pub workspace_root: PathBuf,
    pub cache: Arc<TaglistCache>,
}

#[async_trait]
impl Tool for VaultTaglistTool {
    fn name(&self) -> &'static str {
        "vault:taglist"
    }

    fn description(&self) -> &'static str {
        "Synthesis-only frontmatter tag map: returns tag→count (and optional paths) for notes under 30_Synthesis/. Use to orient before guessing keywords for vault:search; non-synthesis folders are skipped."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VaultTaglistArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: VaultTaglistArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let force_refresh = args.refresh.unwrap_or(false);
        let dirty = self.cache.take_dirty();
        let snapshot_path = crate::vault_layout::taglist_json(&self.workspace_root);
        let snapshot = if force_refresh || dirty || !snapshot_path.exists() {
            let fresh = build_synthesis_taglist(&self.workspace_root).await?;
            persist(&self.workspace_root, &fresh).await?;
            fresh
        } else {
            match load_persisted(&self.workspace_root).await? {
                Some(s) => s,
                None => {
                    let fresh = build_synthesis_taglist(&self.workspace_root).await?;
                    persist(&self.workspace_root, &fresh).await?;
                    fresh
                }
            }
        };

        Ok(render_output(&snapshot, &args))
    }
}

fn render_output(snapshot: &TaglistSnapshot, args: &VaultTaglistArgs) -> String {
    let header = format!(
        "[VAULT_TAGLIST scope=30_Synthesis built_at_unix={} note_count={} tag_count={}]",
        snapshot.built_at_unix,
        snapshot.note_count,
        snapshot.tags.len()
    );

    if let Some(target) = args.tag.as_ref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        return match snapshot.tags.iter().find(|e| e.tag == target) {
            Some(entry) => format!(
                "{header}\nTag: {} (count={})\nPaths:\n{}",
                entry.tag,
                entry.count,
                entry
                    .paths
                    .iter()
                    .map(|p| format!("- {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            None => format!(
                "{header}\nTag: {target} not found in 30_Synthesis frontmatter."
            ),
        };
    }

    let prefix = args
        .prefix
        .as_ref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let min_count = args.min_count.unwrap_or(1).max(1);
    let top_k = args.top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K);
    let include_paths = args.include_paths.unwrap_or(false);

    let filtered: Vec<&TaglistEntry> = snapshot
        .tags
        .iter()
        .filter(|e| e.count >= min_count)
        .filter(|e| match &prefix {
            Some(p) => e.tag.starts_with(p),
            None => true,
        })
        .take(top_k as usize)
        .collect();

    if filtered.is_empty() {
        return format!("{header}\n(no tags match the supplied filters)");
    }

    let mut body_lines: Vec<String> = Vec::with_capacity(filtered.len());
    for entry in filtered {
        if include_paths {
            let paths = entry
                .paths
                .iter()
                .map(|p| format!("    - {p}"))
                .collect::<Vec<_>>()
                .join("\n");
            body_lines.push(format!("- {} ({})\n{}", entry.tag, entry.count, paths));
        } else {
            body_lines.push(format!("- {} ({})", entry.tag, entry.count));
        }
    }
    format!("{header}\n{}", body_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_synth(root: &std::path::Path, node: &str, rev: u32, tags_block: &str) {
        let path = root.join(format!("30_Synthesis/{node}/r{:04}.md", rev));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        let body = format!("---\ntitle: {node}\n{tags_block}\n---\nbody");
        std::fs::write(path, body).expect("write");
    }

    fn make_tool(dir: &std::path::Path) -> VaultTaglistTool {
        VaultTaglistTool {
            workspace_root: dir.to_path_buf(),
            cache: TaglistCache::into_arc(),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cold_call_builds_and_persists() -> Result<()> {
        let dir = tempdir().expect("tmp");
        write_synth(dir.path(), "node-a", 1, "tags:\n  - sandbox\n  - agent");
        write_synth(dir.path(), "node-b", 1, "tags: [agent, topology]");

        let tool = make_tool(dir.path());
        let out = tool.execute(serde_json::json!({})).await?;
        assert!(out.contains("VAULT_TAGLIST"));
        assert!(out.contains("agent (2)"));
        assert!(out.contains("sandbox (1)"));
        assert!(out.contains("topology (1)"));

        let path = crate::vault_layout::taglist_json(dir.path());
        assert!(path.exists());
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn second_call_without_dirty_reads_existing_snapshot() -> Result<()> {
        let dir = tempdir().expect("tmp");
        write_synth(dir.path(), "node-a", 1, "tags:\n  - sandbox");
        let tool = make_tool(dir.path());
        let _ = tool.execute(serde_json::json!({})).await?;

        std::fs::remove_dir_all(dir.path().join("30_Synthesis")).expect("rm");

        let out = tool.execute(serde_json::json!({})).await?;
        assert!(out.contains("sandbox (1)"), "expected stale cached entry, got: {out}");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dirty_flag_forces_rebuild() -> Result<()> {
        let dir = tempdir().expect("tmp");
        write_synth(dir.path(), "node-a", 1, "tags:\n  - sandbox");
        let tool = make_tool(dir.path());
        let _ = tool.execute(serde_json::json!({})).await?;

        write_synth(dir.path(), "node-b", 1, "tags:\n  - new-thing");
        tool.cache.mark_dirty();

        let out = tool.execute(serde_json::json!({})).await?;
        assert!(out.contains("new-thing (1)"));
        assert!(out.contains("sandbox (1)"));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refresh_arg_forces_rebuild_even_when_clean() -> Result<()> {
        let dir = tempdir().expect("tmp");
        write_synth(dir.path(), "node-a", 1, "tags:\n  - sandbox");
        let tool = make_tool(dir.path());
        let _ = tool.execute(serde_json::json!({})).await?;
        write_synth(dir.path(), "node-c", 1, "tags:\n  - newer");

        let out = tool
            .execute(serde_json::json!({ "refresh": true }))
            .await?;
        assert!(out.contains("newer (1)"));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tag_arg_returns_paths() -> Result<()> {
        let dir = tempdir().expect("tmp");
        write_synth(dir.path(), "node-a", 1, "tags:\n  - sandbox");
        write_synth(dir.path(), "node-b", 1, "tags:\n  - sandbox");
        let tool = make_tool(dir.path());

        let out = tool
            .execute(serde_json::json!({ "tag": "Sandbox" }))
            .await?;
        assert!(out.contains("Tag: sandbox"));
        assert!(out.contains("30_Synthesis/node-a/r0001.md"));
        assert!(out.contains("30_Synthesis/node-b/r0001.md"));
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn prefix_and_top_k_filter() -> Result<()> {
        let dir = tempdir().expect("tmp");
        write_synth(dir.path(), "n1", 1, "tags:\n  - agent-loop");
        write_synth(dir.path(), "n2", 1, "tags:\n  - agent-self");
        write_synth(dir.path(), "n3", 1, "tags:\n  - sandbox");
        let tool = make_tool(dir.path());

        let out = tool
            .execute(serde_json::json!({ "prefix": "AGENT" }))
            .await?;
        assert!(out.contains("agent-loop"));
        assert!(out.contains("agent-self"));
        assert!(!out.contains("sandbox"));

        let capped = tool
            .execute(serde_json::json!({ "top_k": 1 }))
            .await?;
        let body = capped
            .lines()
            .filter(|l| l.starts_with("- "))
            .count();
        assert_eq!(body, 1);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn empty_synthesis_returns_empty_snapshot() -> Result<()> {
        let dir = tempdir().expect("tmp");
        let tool = make_tool(dir.path());
        let out = tool.execute(serde_json::json!({})).await?;
        assert!(out.contains("note_count=0"));
        assert!(out.contains("tag_count=0"));
        Ok(())
    }
}
