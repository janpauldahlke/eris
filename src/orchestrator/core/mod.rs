//! Orchestrator core: agent loop, routing, tool dispatch, and condensation.

mod condensation;
mod deck;
mod helpers;
mod llm_directive;
mod orchestrator;
mod pre_llm_routing;
mod step;
mod tool_dispatch;
mod transitions;
mod turn_entry;

pub use orchestrator::Orchestrator;
pub use orchestrator::EMPTY_USER_MESSAGE_TAG;
pub(crate) use orchestrator::{
    PromotionSuppressedDuringStep, TOOL_ROUND_CAP_SYSTEM_GUIDANCE, TOOL_ROUND_CAP_USER_FOOTNOTE,
};

#[cfg(test)]
mod tests;
