//! Debounced vault filesystem watch → live Qdrant re-index for ingest roots.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio_util::sync::CancellationToken;

use crate::memory::semantic::{SemanticBrain, VAULT_INGEST_SUBDIRS_V2};

/// Watches v2 ingest roots and syncs markdown changes into Qdrant.
pub fn spawn_vault_semantic_reindex_watch(
    cancel_token: CancellationToken,
    debounce: Duration,
    vault_root: PathBuf,
    semantic: std::sync::Arc<SemanticBrain>,
) {
    let watch_roots: Vec<PathBuf> = VAULT_INGEST_SUBDIRS_V2
        .iter()
        .map(|subdir| vault_root.join(subdir))
        .filter(|p| p.is_dir())
        .collect();

    if watch_roots.is_empty() {
        tracing::debug!(
            target: "fcp.vault_reindex",
            "no ingest roots on disk; skipping semantic reindex watch"
        );
        return;
    }

    let (quit_tx, quit_rx) = mpsc::channel::<()>();
    let (event_out, mut event_in) = tokio::sync::mpsc::unbounded_channel::<notify::Result<Event>>();

    let roots_for_thread = watch_roots.clone();
    std::thread::spawn(move || {
        let mut watcher = match RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                let _ = event_out.send(res);
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(
                    target: "fcp.vault_reindex",
                    error = %e,
                    "notify init failed; semantic reindex watch disabled"
                );
                return;
            }
        };

        for root in &roots_for_thread {
            if let Err(e) = watcher.watch(root.as_path(), RecursiveMode::Recursive) {
                tracing::error!(
                    target: "fcp.vault_reindex",
                    path = %root.display(),
                    error = %e,
                    "notify watch registration failed"
                );
            }
        }

        let _ = quit_rx.recv();
        drop(watcher);
    });

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    let _ = quit_tx.send(());
                    break;
                }
                next = event_in.recv() => {
                    let Some(res) = next else {
                        let _ = quit_tx.send(());
                        break;
                    };
                    let event = match res {
                        Ok(e) => e,
                        Err(e) => {
                            tracing::warn!(target: "fcp.vault_reindex", error = %e, "notify error");
                            continue;
                        }
                    };

                    let mut rel_paths: HashSet<String> = HashSet::new();
                    for path in event.paths {
                        if let Some(rel) = rel_path_for_reindex(&vault_root, &path) {
                            rel_paths.insert(rel);
                        }
                    }
                    if rel_paths.is_empty() {
                        continue;
                    }

                    tokio::time::sleep(debounce).await;

                    while let Ok(more) = event_in.try_recv() {
                        if let Ok(e) = more {
                            for path in e.paths {
                                if let Some(rel) = rel_path_for_reindex(&vault_root, &path) {
                                    rel_paths.insert(rel);
                                }
                            }
                        }
                    }

                    let is_remove = matches!(
                        event.kind,
                        EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
                    );

                    for rel in rel_paths {
                        if is_remove && !vault_root.join(&rel).exists() {
                            if let Err(e) = semantic.delete_vault_document_v2(&rel).await {
                                tracing::warn!(
                                    target: "fcp.vault_reindex",
                                    vault_key = %rel,
                                    error = %e,
                                    "failed to delete Qdrant point for removed vault file"
                                );
                            } else {
                                tracing::debug!(
                                    target: "fcp.vault_reindex",
                                    vault_key = %rel,
                                    "removed vault file from semantic index"
                                );
                            }
                            continue;
                        }
                        if let Err(e) = semantic.sync_vault_path(&vault_root, &rel).await {
                            tracing::warn!(
                                target: "fcp.vault_reindex",
                                vault_key = %rel,
                                error = %e,
                                "semantic reindex failed"
                            );
                        }
                    }
                }
            }
        }
    });
}

fn rel_path_for_reindex(vault_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(vault_root).ok()?;
    let key = rel.to_string_lossy().replace('\\', "/");
    if !key.ends_with(".md") {
        return None;
    }
    if key.contains("/web/missions/") {
        return None;
    }
    if VAULT_INGEST_SUBDIRS_V2
        .iter()
        .any(|subdir| key.starts_with(&format!("{subdir}/")))
    {
        Some(key)
    } else {
        None
    }
}
