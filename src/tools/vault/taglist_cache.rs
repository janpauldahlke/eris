//! Shared dirty flag bridging `vault:write` and `vault:taglist`. The on-disk snapshot under
//! `.fcp/tools/taglist.json` is the source of truth; this struct just signals "synthesis changed,
//! rebuild on next read".

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct TaglistCache {
    dirty: AtomicBool,
}

impl TaglistCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// Atomically read-and-clear the dirty flag.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_then_take_clears() {
        let c = TaglistCache::new();
        assert!(!c.is_dirty());
        c.mark_dirty();
        assert!(c.is_dirty());
        assert!(c.take_dirty());
        assert!(!c.is_dirty());
        assert!(!c.take_dirty());
    }
}
