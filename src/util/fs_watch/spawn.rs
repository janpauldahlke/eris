use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::executive::vault_identity;
use crate::util::fs_watch::filter::{
    event_paths_match_vault_watch, path_is_under_any_dir, path_matches_any_target,
};

/// Debounced handling of vault watch paths: identity file changes refresh the snapshot; activity under
/// `watched_upload_dirs` is logged at `info` for future hooks (no identity reload unless the identity file changed).
/// On `notify` init or watch registration failure, logs at `error` and returns without spawning (initial snapshot still valid).
pub fn spawn_vault_identity_watch(
    cancel_token: CancellationToken,
    debounce: Duration,
    identity_path: PathBuf,
    watched_files: Vec<PathBuf>,
    watched_upload_dirs: Vec<PathBuf>,
    identity_tx: watch::Sender<Arc<str>>,
) {
    let mut parents: HashSet<PathBuf> = watched_files
        .iter()
        .filter_map(|p| p.parent().map(PathBuf::from))
        .collect();
    let upload_set: HashSet<PathBuf> = watched_upload_dirs.iter().cloned().collect();
    parents.retain(|p| !upload_set.contains(p));

    if parents.is_empty() && watched_upload_dirs.is_empty() {
        tracing::warn!(
            target: "fcp.vault_watch",
            "no watch roots derived from vault_watch.paths; skipping notify"
        );
        return;
    }

    let (quit_tx, quit_rx) = mpsc::channel::<()>();

    let (event_out, mut event_in) = tokio::sync::mpsc::unbounded_channel::<notify::Result<Event>>();

    let parents_vec: Vec<PathBuf> = parents.into_iter().collect();
    let upload_dirs_vec = watched_upload_dirs.clone();

    std::thread::spawn(move || {
        let event_out = event_out;
        let mut watcher = match RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                let _ = event_out.send(res);
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(
                    target: "fcp.vault_watch",
                    error = %e,
                    "notify RecommendedWatcher init failed; identity hot-reload disabled"
                );
                return;
            }
        };

        for parent in &parents_vec {
            if let Err(e) = watcher.watch(parent.as_path(), RecursiveMode::NonRecursive) {
                tracing::error!(
                    target: "fcp.vault_watch",
                    path = %parent.display(),
                    error = %e,
                    "notify watch registration failed"
                );
            }
        }

        for dir in &upload_dirs_vec {
            if let Err(e) = watcher.watch(dir.as_path(), RecursiveMode::Recursive) {
                tracing::error!(
                    target: "fcp.vault_watch",
                    path = %dir.display(),
                    error = %e,
                    "notify recursive watch on user upload dir failed"
                );
            }
        }

        let _ = quit_rx.recv();
        drop(watcher);
    });

    let watched_files_for_task = watched_files;
    let upload_dirs_for_task = watched_upload_dirs;
    tokio::spawn(async move {
        let identity_path = identity_path;

        loop {
            tokio::select! {
                biased;
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
                            tracing::warn!(target: "fcp.vault_watch", error = %e, "notify error event");
                            continue;
                        }
                    };

                    if !event_paths_match_vault_watch(
                        &event.paths,
                        identity_path.as_path(),
                        &watched_files_for_task,
                        &upload_dirs_for_task,
                    ) {
                        continue;
                    }

                    tokio::time::sleep(debounce).await;

                    let mut batch_paths = event.paths;
                    while let Ok(more) = event_in.try_recv() {
                        match more {
                            Ok(e) => batch_paths.extend(e.paths),
                            Err(e) => {
                                tracing::warn!(target: "fcp.vault_watch", error = %e, "notify error in debounce batch");
                            }
                        }
                    }

                    let reload_identity = batch_paths
                        .iter()
                        .any(|p| path_matches_any_target(p.as_path(), &watched_files_for_task));
                    let upload_activity = batch_paths.iter().any(|p| {
                        path_is_under_any_dir(p.as_path(), &upload_dirs_for_task)
                    });

                    if reload_identity {
                        match vault_identity::try_read_identity_for_reload(&identity_path).await {
                            Ok(arc) => {
                                if identity_tx.send(arc.clone()).is_err() {
                                    tracing::warn!(
                                        target: "fcp.vault_watch",
                                        "identity watch: no receivers; stopping reload loop"
                                    );
                                    let _ = quit_tx.send(());
                                    break;
                                }
                                tracing::info!(
                                    target: "fcp.vault_watch",
                                    path = %identity_path.display(),
                                    len = arc.len(),
                                    phase = "debounced_reload",
                                    "identity snapshot refreshed"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "fcp.vault_watch",
                                    path = %identity_path.display(),
                                    error = %e,
                                    phase = "debounced_reload",
                                    "identity reload failed; keeping previous snapshot"
                                );
                            }
                        }
                    }

                    if upload_activity {
                        tracing::info!(
                            target: "fcp.vault_watch",
                            phase = "user_upload_dir_activity",
                            paths = ?batch_paths,
                            "vault 99_USER_UPLOADED (or configured upload roots) changed"
                        );
                    }
                }
            }
        }
    });
}
