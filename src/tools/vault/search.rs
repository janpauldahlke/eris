//! Lexical vault content search (`vault:search`): recursive scan with snippets for LLM summarization.

use async_trait::async_trait;
use regex::RegexBuilder;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::executive::error::{FcpError, Result};
use crate::tools::traits::Tool;

#[derive(Deserialize, JsonSchema)]
pub struct VaultSearchArgs {
    /// Free-text query. Unless `regex` is true: case-insensitive; whitespace splits into terms that must all appear (AND).
    pub query: String,
    /// Optional vault-relative subdirectory (e.g. `10_Topology`). Default: entire vault.
    #[serde(default)]
    pub directory: Option<String>,
    /// When true, `query` is a Rust regex (case-insensitive). Default: false.
    #[serde(default)]
    pub regex: Option<bool>,
    /// Max files to return (clamped to `vault_search_max_files` in config). Default: config.
    #[serde(default)]
    pub max_files: Option<u32>,
}

pub struct VaultSearchTool {
    pub workspace_root: PathBuf,
    pub max_files: u32,
    pub max_snippets_per_file: u32,
    pub snippet_radius_lines: u32,
    pub max_total_chars: usize,
    pub max_file_bytes: u64,
}

struct SearchParams {
    scope_root: PathBuf,
    /// Canonical workspace root; used for `strip_prefix` and traversal checks.
    workspace_root: PathBuf,
    query: String,
    use_regex: bool,
    max_files: usize,
    max_snippets_per_file: usize,
    snippet_radius_lines: usize,
    max_total_chars: usize,
    max_file_bytes: u64,
}

struct FileMatch {
    rel_path: String,
    hit_count: usize,
    excerpt: String,
}

#[async_trait]
impl Tool for VaultSearchTool {
    fn name(&self) -> &'static str {
        "vault:search"
    }

    fn description(&self) -> &'static str {
        "Recursively searches vault .md/.txt file contents for keywords or regex; returns top matching files with line excerpts for summarization. Complements memory:query (semantic) and vault:list (filenames only)."
    }

    fn parameters_schema(&self) -> schemars::schema::RootSchema {
        schemars::schema_for!(VaultSearchArgs)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let args: VaultSearchArgs = serde_json::from_value(args).map_err(FcpError::ParseFault)?;

        let raw_q = args.query.trim();
        if raw_q.is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: "Query cannot be empty".into(),
            });
        }

        let dir_normalized = args
            .directory
            .as_deref()
            .unwrap_or("")
            .trim()
            .replace('\\', "/");
        let dir_trimmed = dir_normalized.trim_end_matches('/');

        let workspace_canon = std::fs::canonicalize(&self.workspace_root).map_err(FcpError::Io)?;

        let scope_root = if dir_trimmed.is_empty() || dir_trimmed == "." {
            workspace_canon.clone()
        } else {
            let joined = self.workspace_root.join(dir_trimmed);
            let scope_canon = std::fs::canonicalize(&joined).map_err(|e| FcpError::ToolFault {
                tool_name: self.name().into(),
                reason: format!("Search scope path is missing or unreadable: {e}"),
            })?;
            if !scope_canon.starts_with(&workspace_canon) {
                return Err(FcpError::ToolFault {
                    tool_name: self.name().into(),
                    reason: "Path Traversal Denied".into(),
                });
            }
            scope_canon
        };

        let max_files_cap = self.max_files.max(1) as usize;
        let requested = args.max_files.unwrap_or(self.max_files).max(1) as usize;
        let max_files = requested.min(max_files_cap);

        let params = SearchParams {
            scope_root,
            workspace_root: workspace_canon,
            query: raw_q.to_string(),
            use_regex: args.regex.unwrap_or(false),
            max_files,
            max_snippets_per_file: self.max_snippets_per_file.max(1) as usize,
            snippet_radius_lines: self.snippet_radius_lines as usize,
            max_total_chars: self.max_total_chars.max(256),
            max_file_bytes: self.max_file_bytes.max(1024),
        };

        tokio::task::spawn_blocking(move || run_vault_search(params))
            .await
            .map_err(|e| FcpError::ToolFault {
                tool_name: "vault:search".into(),
                reason: format!("Blocking task join failed: {e}"),
            })?
    }
}

fn run_vault_search(params: SearchParams) -> Result<String> {
    let workspace_root = &params.workspace_root;
    let mut paths = Vec::new();
    collect_search_files(&params.scope_root, workspace_root, &mut paths)?;

    paths.sort_by(|a, b| {
        rel_path_for_sort(workspace_root, a).cmp(&rel_path_for_sort(workspace_root, b))
    });

    let mut scanned_files: usize = 0;
    let mut matches: Vec<FileMatch> = Vec::new();

    for path in paths {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "vault:search: skip metadata");
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let len = meta.len();
        if len > params.max_file_bytes {
            tracing::debug!(path = %path.display(), len, "vault:search: skip oversized file");
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(path = %path.display(), error = %e, "vault:search: skip unreadable file");
                continue;
            }
        };
        scanned_files += 1;

        let rel = rel_path_string(workspace_root, &path);
        let hit = if params.use_regex {
            match scan_file_regex(&content, &params.query, &rel, &params) {
                Ok(None) => continue,
                Ok(Some(m)) => m,
                Err(e) => return Err(e),
            }
        } else {
            match scan_file_substrings(&content, &params.query, &rel, &params) {
                None => continue,
                Some(m) => m,
            }
        };

        matches.push(hit);
    }

    matches.sort_by(|a, b| match b.hit_count.cmp(&a.hit_count) {
        std::cmp::Ordering::Equal => a.rel_path.cmp(&b.rel_path),
        o => o,
    });

    let matched_total = matches.len();
    let top: Vec<_> = matches.into_iter().take(params.max_files).collect();
    let returned = top.len();

    let mut body = String::new();

    for (idx, m) in top.iter().enumerate() {
        let n = idx + 1;
        body.push_str(&format!(
            "## {}. {}  (matches: {})\n",
            n, m.rel_path, m.hit_count
        ));
        body.push_str(&m.excerpt);
        body.push_str("\n---\n");
    }

    if body.is_empty() {
        body.push_str("No matching vault files found for this query.\n");
    }

    let footer = format!(
        "[vault:search summary: scanned {} files, {} matched; top {} returned. Use vault:read for full content.]",
        scanned_files, matched_total, returned
    );

    let notice_trunc = "\n\n[vault:search: output truncated to configured max_total_chars; narrow with `directory`, fewer terms, or increase vault_search_max_total_chars in .fcp/config.toml.]";

    let mut full = body.clone();
    if !full.ends_with('\n') {
        full.push('\n');
    }
    full.push_str(&footer);

    if full.len() <= params.max_total_chars {
        return Ok(full);
    }

    let headroom = footer.len() + notice_trunc.len() + 8;
    let body_cap = params.max_total_chars.saturating_sub(headroom).max(1);
    let body_trunc: String = body.chars().take(body_cap).collect();
    Ok(format!("{body_trunc}{notice_trunc}\n{footer}"))
}

fn rel_path_string(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.replace('\\', "/"))
        .unwrap_or_else(|| path.display().to_string())
}

fn rel_path_for_sort(workspace_root: &Path, path: &Path) -> String {
    rel_path_string(workspace_root, path)
}

fn collect_search_files(dir: &Path, workspace_root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(dir = %dir.display(), error = %e, "vault:search: read_dir failed");
            return Ok(());
        }
    };

    for entry in read_dir {
        let entry = entry.map_err(FcpError::Io)?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }

        let ft = entry.file_type().map_err(FcpError::Io)?;
        if ft.is_symlink() {
            continue;
        }

        if ft.is_dir() {
            if name == ".fcp" || name == "target" {
                continue;
            }
            if !path.starts_with(workspace_root) {
                continue;
            }
            collect_search_files(&path, workspace_root, out)?;
        } else if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());
            if ext.as_deref() != Some("md") && ext.as_deref() != Some("txt") {
                continue;
            }
            if !path.starts_with(workspace_root) {
                continue;
            }
            out.push(path);
        }
    }
    Ok(())
}

fn scan_file_substrings(
    content: &str,
    query: &str,
    rel_path: &str,
    params: &SearchParams,
) -> Option<FileMatch> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    if terms.is_empty() {
        return None;
    }

    let lower = content.to_lowercase();

    for t in &terms {
        if !lower.contains(t.as_str()) {
            return None;
        }
    }

    let mut hit_count: usize = 0;
    for t in &terms {
        hit_count += lower.matches(t.as_str()).count();
    }

    let lines: Vec<&str> = content.lines().collect();
    let max_line = lines.len().max(1);

    let mut hit_line_nums: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let line_l = line.to_lowercase();
        let line_hit = terms.iter().any(|t| line_l.contains(t.as_str()));
        if line_hit {
            hit_line_nums.push(i + 1);
            if hit_line_nums.len() >= params.max_snippets_per_file {
                break;
            }
        }
    }

    let excerpt = build_excerpt(
        &lines,
        &hit_line_nums,
        max_line,
        params.snippet_radius_lines,
    );

    Some(FileMatch {
        rel_path: rel_path.to_string(),
        hit_count,
        excerpt,
    })
}

fn scan_file_regex(
    content: &str,
    query: &str,
    rel_path: &str,
    params: &SearchParams,
) -> Result<Option<FileMatch>> {
    let re = match RegexBuilder::new(query).case_insensitive(true).build() {
        Ok(r) => r,
        Err(e) => {
            return Err(FcpError::ToolFault {
                tool_name: "vault:search".into(),
                reason: format!("Invalid regex: {e}"),
            });
        }
    };

    let hit_count = re.find_iter(content).count();
    if hit_count == 0 {
        return Ok(None);
    }

    let lines: Vec<&str> = content.lines().collect();
    let max_line = lines.len().max(1);

    let mut hit_line_nums: Vec<usize> = Vec::new();
    for cap in re.find_iter(content) {
        let line_no = line_number_at_byte_pos(content, cap.start());
        if !hit_line_nums.contains(&line_no) {
            hit_line_nums.push(line_no);
        }
        if hit_line_nums.len() >= params.max_snippets_per_file {
            break;
        }
    }

    hit_line_nums.sort_unstable();

    let excerpt = build_excerpt(
        &lines,
        &hit_line_nums,
        max_line,
        params.snippet_radius_lines,
    );

    Ok(Some(FileMatch {
        rel_path: rel_path.to_string(),
        hit_count,
        excerpt,
    }))
}

fn line_number_at_byte_pos(content: &str, byte_pos: usize) -> usize {
    let end = byte_pos.min(content.len());
    1 + content[..end].bytes().filter(|&b| b == b'\n').count()
}

fn build_excerpt(lines: &[&str], hit_centers: &[usize], max_line: usize, radius: usize) -> String {
    let mut show_lines: BTreeSet<usize> = BTreeSet::new();
    for &center in hit_centers {
        let start = center.saturating_sub(radius).max(1);
        let end = (center + radius).min(max_line);
        for ln in start..=end {
            show_lines.insert(ln);
        }
    }

    let mut s = String::new();
    for ln in show_lines {
        let idx = ln.saturating_sub(1);
        let text = lines.get(idx).copied().unwrap_or("");
        s.push_str(&format!("> L{}: {}\n", ln, text));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs;

    fn tool(root: &Path) -> VaultSearchTool {
        VaultSearchTool {
            workspace_root: root.to_path_buf(),
            max_files: 10,
            max_snippets_per_file: 3,
            snippet_radius_lines: 1,
            max_total_chars: 12_000,
            max_file_bytes: 1_048_576,
        }
    }

    #[tokio::test]
    async fn recursive_match_ranks_by_hits() -> Result<()> {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a")).await.unwrap();
        fs::create_dir_all(dir.path().join("b")).await.unwrap();
        fs::write(dir.path().join("a/one.md"), "foo and foo again\n")
            .await
            .unwrap();
        fs::write(dir.path().join("b/two.md"), "foo\n")
            .await
            .unwrap();

        let t = tool(dir.path());
        let args = serde_json::json!({ "query": "foo" });
        let out = t.execute(args).await?;
        assert!(out.contains("a/one.md"));
        let pos_one = out.find("a/one.md").unwrap_or(usize::MAX);
        let pos_two = out.find("b/two.md").unwrap_or(usize::MAX);
        assert!(pos_one < pos_two, "denser file should rank first: {}", out);
        Ok(())
    }

    #[tokio::test]
    async fn case_insensitive_default() -> Result<()> {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("n.md"),
            "We discussed database migration here.\n",
        )
        .await
        .unwrap();

        let t = tool(dir.path());
        let args = serde_json::json!({ "query": "DATABASE" });
        let out = t.execute(args).await?;
        assert!(out.contains("database migration"));
        Ok(())
    }

    #[tokio::test]
    async fn respects_directory_scope() -> Result<()> {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("10_Topology"))
            .await
            .unwrap();
        fs::create_dir_all(dir.path().join("20_Other"))
            .await
            .unwrap();
        fs::write(dir.path().join("10_Topology/a.md"), "uniquemarker xyz\n")
            .await
            .unwrap();
        fs::write(dir.path().join("20_Other/b.md"), "uniquemarker xyz\n")
            .await
            .unwrap();

        let t = tool(dir.path());
        let args = serde_json::json!({
            "query": "uniquemarker",
            "directory": "10_Topology"
        });
        let out = t.execute(args).await?;
        assert!(out.contains("10_Topology/a.md"));
        assert!(!out.contains("20_Other/b.md"));
        Ok(())
    }

    #[tokio::test]
    async fn skips_dotfcp_and_oversized() -> Result<()> {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".fcp/telemetry/logs"))
            .await
            .unwrap();
        fs::write(
            dir.path().join(".fcp/telemetry/logs/x.md"),
            "SECRET_DOT_FCP matchme\n",
        )
        .await
        .unwrap();

        let huge_path = dir.path().join("huge.md");
        let mut huge = String::with_capacity(2_000_000);
        huge.push_str("padding\n");
        huge.push_str(&"a".repeat(2_000_000));
        huge.push_str("\nmatchme_tail\n");
        fs::write(&huge_path, huge).await.unwrap();

        fs::write(dir.path().join("ok.md"), "matchme visible\n")
            .await
            .unwrap();

        let t = tool(dir.path());
        let args = serde_json::json!({ "query": "matchme" });
        let out = t.execute(args).await?;
        assert!(out.contains("ok.md"));
        assert!(!out.contains(".fcp/"));
        assert!(!out.contains("huge.md"));
        Ok(())
    }

    #[tokio::test]
    async fn path_traversal_denied() -> Result<()> {
        let dir = tempdir().unwrap();
        let t = tool(dir.path());
        let args = serde_json::json!({
            "query": "x",
            "directory": "../../etc"
        });
        let err = t.execute(args).await.unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("Traversal")
                || s.contains("traversal")
                || s.contains("missing or unreadable"),
            "expected traversal-style rejection or invalid scope rejection, got: {}",
            s
        );
        Ok(())
    }

    #[tokio::test]
    async fn total_char_budget_truncates() -> Result<()> {
        let dir = tempdir().unwrap();
        for i in 0..15 {
            fs::write(
                dir.path().join(format!("f{i}.md")),
                "keyword repeated keyword keyword\n",
            )
            .await
            .unwrap();
        }

        let t = VaultSearchTool {
            workspace_root: dir.path().to_path_buf(),
            max_files: 10,
            max_snippets_per_file: 3,
            snippet_radius_lines: 1,
            max_total_chars: 800,
            max_file_bytes: 1_048_576,
        };
        let args = serde_json::json!({ "query": "keyword" });
        let out = t.execute(args).await?;
        assert!(out.len() <= 900);
        assert!(out.contains("truncated"));
        Ok(())
    }

    #[tokio::test]
    async fn regex_flag_compiles() -> Result<()> {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("r.md"), "code 123 and 456\n")
            .await
            .unwrap();

        let t = tool(dir.path());
        let args = serde_json::json!({
            "query": r"\d{3}",
            "regex": true
        });
        let out = t.execute(args).await?;
        assert!(out.contains("123"));

        let bad = serde_json::json!({
            "query": "[",
            "regex": true
        });
        let err = t.execute(bad).await.unwrap_err();
        assert!(format!("{err}").contains("regex") || format!("{err}").contains("Invalid"));
        Ok(())
    }
}
