//! Session-scoped short handles (`buf_1`, `buf_2`, …) mapped to canonical ephemeral `staged_id` (UUID).
//! LLM-facing tool args use the handle; storage and vector indexing keep the UUID.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct BufferHandleRegistry {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    next_seq: u32,
    handles_to_staged: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferHandleResolveError {
    Empty,
    UnknownHandle,
}

impl BufferHandleRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
        }
    }

    /// Register a canonical `staged_id` and return a fresh handle for tool arguments and receipts.
    pub async fn register(&self, staged_id: String) -> String {
        let mut g = self.inner.write().await;
        g.next_seq = g.next_seq.saturating_add(1);
        let handle = format!("buf_{}", g.next_seq);
        tracing::debug!(
            handle = %handle,
            staged_id = %staged_id,
            "buffer handle mapped to canonical staged_id"
        );
        g.handles_to_staged.insert(handle.clone(), staged_id);
        handle
    }

    /// Resolve an agent-supplied `buffer_id`: session handle or legacy raw UUID.
    pub async fn resolve_for_lookup(&self, input: &str) -> Result<String, BufferHandleResolveError> {
        let t = input.trim();
        if t.is_empty() {
            return Err(BufferHandleResolveError::Empty);
        }
        {
            let g = self.inner.read().await;
            if let Some(id) = g.handles_to_staged.get(t) {
                return Ok(id.clone());
            }
        }
        if uuid::Uuid::parse_str(t).is_ok() {
            return Ok(t.to_string());
        }
        Err(BufferHandleResolveError::UnknownHandle)
    }
}

impl Default for BufferHandleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_monotonic_handles() {
        let r = BufferHandleRegistry::new();
        let h1 = r.register("uuid-a".into()).await;
        let h2 = r.register("uuid-b".into()).await;
        assert_eq!(h1, "buf_1");
        assert_eq!(h2, "buf_2");
        assert_eq!(
            r.resolve_for_lookup(&h1).await.expect("a"),
            "uuid-a"
        );
        assert_eq!(
            r.resolve_for_lookup(&h2).await.expect("b"),
            "uuid-b"
        );
    }

    #[tokio::test]
    async fn resolve_accepts_raw_uuid_for_back_compat() {
        let r = BufferHandleRegistry::new();
        let u = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(r.resolve_for_lookup(u).await.expect("uuid"), u);
    }

    #[tokio::test]
    async fn unknown_handle_errors() {
        let r = BufferHandleRegistry::new();
        assert_eq!(
            r.resolve_for_lookup("buf_99").await.expect_err("unknown"),
            BufferHandleResolveError::UnknownHandle
        );
    }
}
