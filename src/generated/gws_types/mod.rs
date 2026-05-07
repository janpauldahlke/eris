pub mod calendar;
pub mod gmail;
mod serde_helpers;

pub use gmail::{ActionDescriptor, ParamDescriptor};
pub use serde_helpers::*;

/// Returns Gmail API action descriptors (generated). Calendar descriptors use a distinct struct type in `calendar.rs`.
pub fn all_actions() -> Vec<&'static ActionDescriptor> {
    gmail::ALL_ACTIONS.to_vec()
}
