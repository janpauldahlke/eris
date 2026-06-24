//! Single-worker queue for document ingestion — one embed storm at a time on limited VRAM.
//!
//! Exposes a `watch` channel so the chat session main loop can `select!` on ingest
//! completion and defer LLM inference until the GPU is free.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::{mpsc, oneshot, watch};

use crate::executive::error::{FcpError, Result};
use crate::memory::document_store::{DocumentStore, IngestReceipt};
use crate::presentation::SessionEvent;

const JOB_CHANNEL_CAPACITY: usize = 32;

#[derive(Debug, Clone, Serialize)]
pub struct IngestQueueStatus {
    pub running: bool,
    pub current_path: Option<String>,
    pub queued_paths: Vec<String>,
    pub queued_count: usize,
}

#[derive(Debug)]
pub struct EnqueueReceipt {
    pub queue_position: usize,
}

struct IngestJob {
    relative_path: String,
    label: Option<String>,
    notify_on_complete: bool,
    result_tx: Option<oneshot::Sender<Result<IngestReceipt>>>,
}

#[derive(Clone)]
pub struct DocumentIngestQueue {
    job_tx: mpsc::Sender<IngestJob>,
    /// Latest snapshot of queue status (updated by worker on every transition).
    status_rx: watch::Receiver<IngestQueueStatus>,
    /// True while the queue is actively embedding or has pending jobs (GPU contention possible).
    busy_rx: watch::Receiver<bool>,
}

impl DocumentIngestQueue {
    pub fn spawn(
        store: Arc<DocumentStore>,
        vault_root: PathBuf,
        presentation_tx: Option<mpsc::Sender<SessionEvent>>,
    ) -> Arc<Self> {
        let (job_tx, mut job_rx) = mpsc::channel::<IngestJob>(JOB_CHANNEL_CAPACITY);
        let (busy_tx, busy_rx) = watch::channel(false);
        let initial_status = IngestQueueStatus {
            running: false,
            current_path: None,
            queued_paths: Vec::new(),
            queued_count: 0,
        };
        let (status_tx, status_rx) = watch::channel(initial_status);

        tokio::spawn(async move {
            let mut pending: VecDeque<IngestJob> = VecDeque::new();

            loop {
                // Get next job: either from local buffer or by waiting on channel.
                let job = if let Some(buffered) = pending.pop_front() {
                    buffered
                } else {
                    match job_rx.recv().await {
                        Some(j) => j,
                        None => break,
                    }
                };

                // Drain any additional buffered jobs that arrived while we were idle.
                while let Ok(extra) = job_rx.try_recv() {
                    pending.push_back(extra);
                }

                // Publish: processing this job, pending has the rest.
                let _ = busy_tx.send(true);
                let _ = status_tx.send(IngestQueueStatus {
                    running: true,
                    current_path: Some(job.relative_path.clone()),
                    queued_paths: pending.iter().map(|j| j.relative_path.clone()).collect(),
                    queued_count: pending.len(),
                });

                tracing::info!(
                    event = "fcp.document_ingest.started",
                    path = %job.relative_path,
                    queued_behind = pending.len(),
                    "Document ingest worker started job"
                );

                let result = store
                    .ingest_document(&vault_root, &job.relative_path, job.label.as_deref())
                    .await;

                match &result {
                    Ok(receipt) if !receipt.skipped_unchanged => {
                        tracing::info!(
                            event = "fcp.document_ingest.completed",
                            path = %job.relative_path,
                            doc_id = %receipt.doc_id,
                            chunks = receipt.total_chunks,
                            "Document ingested"
                        );
                    }
                    Ok(receipt) => {
                        tracing::info!(
                            event = "fcp.document_ingest.skipped_unchanged",
                            path = %job.relative_path,
                            doc_id = %receipt.doc_id,
                            "Document unchanged — ingest skipped"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            event = "fcp.document_ingest.failed",
                            path = %job.relative_path,
                            error = %e,
                            "Document ingest failed"
                        );
                    }
                }

                if job.notify_on_complete && let Some(tx) = presentation_tx.as_ref() {
                    let msg = match &result {
                        Ok(receipt) if receipt.skipped_unchanged => format!(
                            "[doc] Unchanged — skipped re-ingest: {} (doc_id {}).",
                            receipt.source_name, receipt.doc_id
                        ),
                        Ok(receipt) => format!(
                            "[doc] Ingested {} ({} chunks, doc_id {}). Use doc:query to search it.",
                            receipt.source_name, receipt.total_chunks, receipt.doc_id
                        ),
                        Err(e) => format!(
                            "[doc] Ingest failed for {}: {e}",
                            job.relative_path
                        ),
                    };
                    if tx.send(SessionEvent::UiNotice(msg)).await.is_err() {
                        tracing::debug!(
                            event = "fcp.document_ingest.notify_dropped",
                            path = %job.relative_path,
                            "Ingest completion UI notice not delivered"
                        );
                    }
                }

                if let Some(tx) = job.result_tx {
                    let _ = tx.send(result);
                }

                // Publish idle state.
                let still_busy = !pending.is_empty();
                let _ = busy_tx.send(still_busy);
                let _ = status_tx.send(IngestQueueStatus {
                    running: false,
                    current_path: None,
                    queued_paths: pending.iter().map(|j| j.relative_path.clone()).collect(),
                    queued_count: pending.len(),
                });
            }
        });

        Arc::new(Self {
            job_tx,
            status_rx,
            busy_rx,
        })
    }

    /// Current queue snapshot (non-blocking read of latest watch value).
    pub fn status(&self) -> IngestQueueStatus {
        self.status_rx.borrow().clone()
    }

    /// True if the worker is actively embedding or jobs remain queued.
    pub fn is_busy(&self) -> bool {
        *self.busy_rx.borrow()
    }

    /// Subscribe to the busy signal. The chat session main loop uses `.changed()` on this
    /// to wake up when ingest finishes and deferred user inputs can be released to the LLM.
    pub fn subscribe_busy(&self) -> watch::Receiver<bool> {
        self.busy_rx.clone()
    }

    /// Queue background ingest (web auto-ingest). Returns queue position (1 = next up).
    pub async fn enqueue_background(
        &self,
        relative_path: String,
        label: Option<String>,
    ) -> Result<EnqueueReceipt> {
        self.send_job(relative_path, label, true, None).await
    }

    /// Queue ingest and wait for this job to finish (`doc:ingest` tool).
    pub async fn ingest_and_wait(
        &self,
        relative_path: String,
        label: Option<String>,
    ) -> Result<IngestReceipt> {
        let (tx, rx) = oneshot::channel();
        self.send_job(relative_path, label, false, Some(tx)).await?;
        rx.await.map_err(|_| FcpError::ToolFault {
            tool_name: "doc:ingest".into(),
            reason: "ingest worker dropped job".into(),
        })?
    }

    async fn send_job(
        &self,
        relative_path: String,
        label: Option<String>,
        notify_on_complete: bool,
        result_tx: Option<oneshot::Sender<Result<IngestReceipt>>>,
    ) -> Result<EnqueueReceipt> {
        let rel = relative_path.replace('\\', "/");
        if rel.trim().is_empty() {
            return Err(FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: "relative_path is required".into(),
            });
        }

        {
            let st = self.status_rx.borrow();
            if st.current_path.as_deref() == Some(rel.as_str())
                || st.queued_paths.iter().any(|p| p == &rel)
            {
                return Err(FcpError::ToolFault {
                    tool_name: "doc:ingest".into(),
                    reason: format!("ingest already queued for {rel}"),
                });
            }
        }

        let job = IngestJob {
            relative_path: rel.clone(),
            label,
            notify_on_complete,
            result_tx,
        };

        self.job_tx
            .send(job)
            .await
            .map_err(|_| FcpError::ToolFault {
                tool_name: "doc:ingest".into(),
                reason: "document ingest queue is closed".into(),
            })?;

        tracing::info!(
            event = "fcp.document_ingest.queued",
            path = %rel,
            "Document ingest queued"
        );

        let queue_position = self.status_rx.borrow().queued_count + 1;
        Ok(EnqueueReceipt { queue_position })
    }
}
