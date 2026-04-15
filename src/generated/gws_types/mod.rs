pub mod gmail;
pub mod calendar;
mod serde_helpers;

pub use serde_helpers::*;
pub use gmail::{ActionDescriptor, ParamDescriptor};

/// Returns Gmail API action descriptors (generated). Calendar descriptors use a distinct struct type in `calendar.rs`.
pub fn all_actions() -> Vec<&'static ActionDescriptor> {
    gmail::ALL_ACTIONS.to_vec()
}