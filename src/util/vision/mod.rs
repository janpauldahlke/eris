//! Server-side image normalization for the vision upload pipeline.

mod normalize;
mod persist;

pub use normalize::{NormalizedImage, normalize_upload};
pub use persist::persist_normalized_image;
