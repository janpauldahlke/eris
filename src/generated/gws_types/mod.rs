pub mod gmail;
mod serde_helpers;

pub use serde_helpers::*;
pub use gmail::{ActionDescriptor, ParamDescriptor};

/// Returns all action descriptors across generated services.
pub fn all_actions() -> Vec<&'static ActionDescriptor> {
    let mut all = Vec::new();
    all.extend_from_slice(gmail::ALL_ACTIONS);
    all
}