//! Pure path matching for notify events (no I/O in unit tests beyond optional canonicalize).

use std::path::{Path, PathBuf};

/// Returns true if any `event_path` refers to the same file as one of `targets`.
pub fn event_paths_match_targets(event_paths: &[PathBuf], targets: &[PathBuf]) -> bool {
    for ep in event_paths {
        if path_matches_any_target(ep.as_path(), targets) {
            return true;
        }
    }
    false
}

/// True when any event path is the identity file, matches another watched file target, or lies under a watched upload directory.
pub fn event_paths_match_vault_watch(
    event_paths: &[PathBuf],
    identity_path: &Path,
    watched_files: &[PathBuf],
    upload_dir_roots: &[PathBuf],
) -> bool {
    for ep in event_paths {
        let p = ep.as_path();
        if paths_refer_to_same_file(p, identity_path) {
            return true;
        }
        if path_matches_any_target(p, watched_files) {
            return true;
        }
        if path_is_under_any_dir(p, upload_dir_roots) {
            return true;
        }
    }
    false
}

/// True if `path` is the identity file (same path or same inode after canonicalize when possible).
pub fn path_touches_identity_file(path: &Path, identity_path: &Path) -> bool {
    paths_refer_to_same_file(path, identity_path)
}

/// True if `path` is `dir` or a descendant (component-wise prefix).
pub fn path_is_under_any_dir(path: &Path, dirs: &[PathBuf]) -> bool {
    for d in dirs {
        if path == d.as_path() {
            return true;
        }
        if path.starts_with(d) {
            return true;
        }
    }
    false
}

pub(crate) fn path_matches_any_target(event: &Path, targets: &[PathBuf]) -> bool {
    for t in targets {
        if paths_refer_to_same_file(event, t.as_path()) {
            return true;
        }
    }
    false
}

fn paths_refer_to_same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => {
            // Best-effort when canonicalize fails (deleted/racy paths).
            a.file_name()
                .zip(b.file_name())
                .is_some_and(|(fa, fb)| fa == fb && a.parent() == b.parent())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_paths_match() {
        let p = PathBuf::from("/tmp/a/Identity.md");
        let targets = [p.clone()];
        assert!(event_paths_match_targets(std::slice::from_ref(&p), &targets));
    }

    #[test]
    fn different_basenames_no_canonical_no_match() {
        let a = PathBuf::from("/x/a.md");
        let b = PathBuf::from("/x/b.md");
        assert!(!event_paths_match_targets(&[a], &[b]));
    }

    #[test]
    fn path_under_upload_dir_matches() {
        let root = PathBuf::from("/vault/ws/99_USER_UPLOADED");
        let file = PathBuf::from("/vault/ws/99_USER_UPLOADED/drop.bin");
        assert!(path_is_under_any_dir(file.as_path(), std::slice::from_ref(&root)));
    }

    #[test]
    fn event_paths_match_vault_watch_upload_only() {
        let id = PathBuf::from("/vault/ws/00_Invariants/Identity.md");
        let upload = PathBuf::from("/vault/ws/99_USER_UPLOADED");
        let ev = PathBuf::from("/vault/ws/99_USER_UPLOADED/x.txt");
        assert!(event_paths_match_vault_watch(
            std::slice::from_ref(&ev),
            id.as_path(),
            std::slice::from_ref(&id),
            std::slice::from_ref(&upload),
        ));
    }
}
