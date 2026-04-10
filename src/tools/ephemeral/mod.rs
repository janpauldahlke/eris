//! Staged **big blob** tools: paging and in-buffer search over [`crate::memory::buffer::BufferedBlob`] rows in ephemeral memory.

pub mod buffer_page;
pub mod buffer_query;

pub use buffer_page::{EphemeralBufferPageArgs, EphemeralBufferPageTool};
pub use buffer_query::{EphemeralBufferQueryArgs, EphemeralBufferQueryTool};
