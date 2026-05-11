//! Synthesis-only frontmatter tag index for `vault:taglist`. Walks `30_Synthesis/<node>/rXXXX.md`,
//! picks the highest revision per node, parses YAML frontmatter `tags` (bullet AND inline list),
//! and persists a `tag -> {count, paths}` snapshot under `.fcp/tools/taglist.json`.
//!
//! Kept independent from `crate::memory::semantic::parse_vault_md` so the new inline-list parsing
//! does not perturb the semantic ingest path.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::executive::error::{FcpError, Result};

const SYNTHESIS_DIR: &str = "30_Synthesis";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaglistEntry {
    pub tag: String,
    pub count: u32,
    /// Synthesis-relative paths (forward slashes, stable across platforms) that carry this tag.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaglistSnapshot {
    pub built_at_unix: u64,
    pub note_count: u32,
    /// Sorted by `count` descending, then `tag` ascending.
    pub tags: Vec<TaglistEntry>,
}

impl TaglistSnapshot {
    pub fn empty() -> Self {
        Self {
            built_at_unix: unix_now_secs(),
            note_count: 0,
            tags: Vec::new(),
        }
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Walks `<workspace_root>/30_Synthesis/` synchronously inside `spawn_blocking` so we never starve
/// the runtime when the vault has many notes. Other roots are intentionally ignored — only
/// synthesis uses reliable frontmatter today.
pub async fn build_synthesis_taglist(workspace_root: &Path) -> Result<TaglistSnapshot> {
    let root = workspace_root.join(SYNTHESIS_DIR);
    if !root.exists() {
        return Ok(TaglistSnapshot::empty());
    }
    let root_for_block = root.clone();
    let snapshot = tokio::task::spawn_blocking(move || build_blocking(&root_for_block))
        .await
        .map_err(|e| FcpError::Config(format!("vault:taglist build join error: {e}")))??;
    Ok(snapshot)
}

fn build_blocking(synthesis_root: &Path) -> Result<TaglistSnapshot> {
    let mut grouped: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut note_count: u32 = 0;

    let read_dir = match std::fs::read_dir(synthesis_root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TaglistSnapshot::empty());
        }
        Err(e) => return Err(FcpError::Io(e)),
    };

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(error = %e, "vault:taglist: skip unreadable synthesis entry");
                continue;
            }
        };
        let node_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let node_id = match node_path.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() && !s.starts_with('.') => s.to_string(),
            _ => continue,
        };

        let head = match highest_revision_in(&node_path) {
            Some(h) => h,
            None => continue,
        };

        let raw = match std::fs::read_to_string(&head) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(path = %head.display(), error = %e, "vault:taglist: failed to read head");
                continue;
            }
        };

        let tags = parse_frontmatter_tags(&raw);
        if tags.is_empty() {
            continue;
        }
        note_count += 1;

        let file_name = head
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("r0001.md");
        let rel = format!("{SYNTHESIS_DIR}/{node_id}/{file_name}");

        for tag in tags {
            grouped
                .entry(tag)
                .or_default()
                .push(rel.clone());
        }
    }

    let mut entries: Vec<TaglistEntry> = grouped
        .into_iter()
        .map(|(tag, mut paths)| {
            paths.sort();
            paths.dedup();
            let count = paths.len() as u32;
            TaglistEntry { tag, count, paths }
        })
        .collect();
    entries.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));

    Ok(TaglistSnapshot {
        built_at_unix: unix_now_secs(),
        note_count,
        tags: entries,
    })
}

fn highest_revision_in(node_dir: &Path) -> Option<PathBuf> {
    let read_dir = std::fs::read_dir(node_dir).ok()?;
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let Some(stripped) = name_str.strip_prefix('r').and_then(|s| s.strip_suffix(".md")) else {
            continue;
        };
        let n = match stripped.parse::<u32>() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if best.as_ref().is_none_or(|(b, _)| n > *b) {
            best = Some((n, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// Extract normalized frontmatter `tags` (lowercase, trimmed, deduped per-note). Supports both
/// bullet form (`tags:\n  - a\n  - b`) and YAML inline form (`tags: [a, "b c", d]`).
pub fn parse_frontmatter_tags(raw: &str) -> Vec<String> {
    let frontmatter = match extract_frontmatter(raw) {
        Some(fm) => fm,
        None => return Vec::new(),
    };

    let mut tags: Vec<String> = Vec::new();
    let mut in_bullets = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim_end();
        let leading_indent = trimmed.len() - trimmed.trim_start().len();
        let body = trimmed.trim_start();

        if let Some(rest) = body.strip_prefix("tags:") {
            in_bullets = false;
            let rest = rest.trim();
            if rest.is_empty() {
                in_bullets = true;
                continue;
            }
            if let Some(inline) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                for chunk in split_inline_list(inline) {
                    push_normalized(&mut tags, &chunk);
                }
            } else {
                push_normalized(&mut tags, rest);
            }
            continue;
        }

        if in_bullets {
            if leading_indent == 0 && !body.starts_with("- ") {
                in_bullets = false;
                continue;
            }
            if let Some(item) = body.strip_prefix("- ") {
                push_normalized(&mut tags, item);
            } else {
                in_bullets = false;
            }
        }
    }

    tags.sort();
    tags.dedup();
    tags
}

fn extract_frontmatter(raw: &str) -> Option<&str> {
    let rest = raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

fn split_inline_list(inline: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_quotes = false;
    let mut quote_char = '\0';
    for c in inline.chars() {
        match c {
            '"' | '\'' if !in_quotes => {
                in_quotes = true;
                quote_char = c;
            }
            c if in_quotes && c == quote_char => {
                in_quotes = false;
            }
            ',' if !in_quotes => {
                out.push(buf.trim().to_string());
                buf.clear();
            }
            other => buf.push(other),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

fn push_normalized(out: &mut Vec<String>, raw: &str) {
    let cleaned = raw
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'')
        .trim()
        .to_lowercase();
    if cleaned.is_empty() {
        return;
    }
    out.push(cleaned);
}

pub async fn load_persisted(workspace_root: &Path) -> Result<Option<TaglistSnapshot>> {
    let path = crate::vault_layout::taglist_json(workspace_root);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(FcpError::Io(e)),
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<TaglistSnapshot>(&raw) {
        Ok(s) => Ok(Some(s)),
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "vault:taglist: persisted snapshot is malformed; treating as missing"
            );
            Ok(None)
        }
    }
}

pub async fn persist(workspace_root: &Path, snapshot: &TaglistSnapshot) -> Result<()> {
    let path = crate::vault_layout::taglist_json(workspace_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(FcpError::Io)?;
    }
    let body = serde_json::to_string_pretty(snapshot).map_err(|e| FcpError::Config(e.to_string()))?;
    tokio::fs::write(&path, body).await.map_err(FcpError::Io)?;
    Ok(())
}

/// Convenience used by chat startup: build + persist, never panicking; logs and returns the
/// snapshot count via `Result` so callers can surface failures via `tracing`.
pub async fn build_and_persist(workspace_root: &Path) -> Result<TaglistSnapshot> {
    let snap = build_synthesis_taglist(workspace_root).await?;
    persist(workspace_root, &snap).await?;
    Ok(snap)
}

/// `true` when a path that just landed on disk should invalidate the cached taglist.
/// Synthesis is a node-keyed `30_Synthesis/<uuid>/rXXXX.md` layout; we accept any `.md` under that
/// prefix.
pub fn is_synthesis_md_path(relative_path: &str) -> bool {
    if !relative_path.ends_with(".md") {
        return false;
    }
    let normalized = relative_path.replace('\\', "/");
    normalized.starts_with(&format!("{SYNTHESIS_DIR}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, body).expect("write");
    }

    #[test]
    fn parse_frontmatter_tags_bullet_style() {
        let raw = "---\ntitle: foo\ntags:\n  - Sandbox\n  - Agent\n---\nbody";
        assert_eq!(parse_frontmatter_tags(raw), vec!["agent".to_string(), "sandbox".to_string()]);
    }

    #[test]
    fn parse_frontmatter_tags_inline_list() {
        let raw = "---\ntitle: foo\ntags: [Sandbox, \"agent loop\", topology]\n---\nbody";
        let mut got = parse_frontmatter_tags(raw);
        got.sort();
        assert_eq!(got, vec!["agent loop".to_string(), "sandbox".to_string(), "topology".to_string()]);
    }

    #[test]
    fn parse_frontmatter_tags_no_frontmatter_returns_empty() {
        let raw = "no fences here";
        assert!(parse_frontmatter_tags(raw).is_empty());
    }

    #[test]
    fn parse_frontmatter_tags_inline_single_value() {
        let raw = "---\ntags: solo\n---\nbody";
        assert_eq!(parse_frontmatter_tags(raw), vec!["solo".to_string()]);
    }

    #[test]
    fn is_synthesis_md_path_detects_canonical() {
        assert!(is_synthesis_md_path("30_Synthesis/uuid/r0003.md"));
        assert!(is_synthesis_md_path("30_Synthesis/uuid/r0001.md"));
        assert!(!is_synthesis_md_path("20_Discourse/Tasks.md"));
        assert!(!is_synthesis_md_path("30_Synthesis/uuid/notes.txt"));
    }

    #[tokio::test]
    async fn build_picks_highest_revision_and_groups_paths() {
        let dir = tempdir().expect("tmpdir");
        let root = dir.path();

        write_file(
            &root.join("30_Synthesis/node-a/r0001.md"),
            "---\ntitle: a-old\ntags:\n  - sandbox\n  - obsolete\n---\nold body",
        );
        write_file(
            &root.join("30_Synthesis/node-a/r0002.md"),
            "---\ntitle: a-new\ntags:\n  - sandbox\n  - agent\n---\nnew body",
        );
        write_file(
            &root.join("30_Synthesis/node-b/r0001.md"),
            "---\ntitle: b\ntags: [Topology, agent]\n---\nbody",
        );
        write_file(
            &root.join("30_Synthesis/.hidden/r0001.md"),
            "---\ntags: [shouldnotcount]\n---\nx",
        );
        write_file(
            &root.join("20_Discourse/notes.md"),
            "---\ntags: [ignored]\n---\nx",
        );

        let snap = build_synthesis_taglist(root).await.expect("build");

        assert_eq!(snap.note_count, 2);
        let by_tag: std::collections::HashMap<&str, &TaglistEntry> =
            snap.tags.iter().map(|e| (e.tag.as_str(), e)).collect();

        let agent = by_tag.get("agent").expect("agent tag");
        assert_eq!(agent.count, 2);
        assert!(agent.paths.iter().any(|p| p == "30_Synthesis/node-a/r0002.md"));
        assert!(agent.paths.iter().any(|p| p == "30_Synthesis/node-b/r0001.md"));
        assert!(!agent.paths.iter().any(|p| p == "30_Synthesis/node-a/r0001.md"));

        let sandbox = by_tag.get("sandbox").expect("sandbox tag");
        assert_eq!(sandbox.count, 1);
        assert_eq!(sandbox.paths, vec!["30_Synthesis/node-a/r0002.md".to_string()]);

        assert!(by_tag.get("obsolete").is_none(), "older revision must be ignored");
        assert!(by_tag.get("shouldnotcount").is_none(), "dot dirs must be skipped");
        assert!(by_tag.get("ignored").is_none(), "non-synthesis roots must be skipped");

        assert!(snap.tags.windows(2).all(|w| w[0].count >= w[1].count));
    }

    #[tokio::test]
    async fn persist_and_load_roundtrip() {
        let dir = tempdir().expect("tmpdir");
        let snap = TaglistSnapshot {
            built_at_unix: 42,
            note_count: 1,
            tags: vec![TaglistEntry {
                tag: "x".into(),
                count: 1,
                paths: vec!["30_Synthesis/n/r0001.md".into()],
            }],
        };
        persist(dir.path(), &snap).await.expect("persist");
        let back = load_persisted(dir.path()).await.expect("load").expect("some");
        assert_eq!(back.note_count, 1);
        assert_eq!(back.tags[0].tag, "x");
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let dir = tempdir().expect("tmpdir");
        let got = load_persisted(dir.path()).await.expect("load");
        assert!(got.is_none());
    }
}
