//! Injected LLM-facing copy (JSON repair hints, post-tool guidance) and helpers for splitting the
//! model’s JSON envelope from trailing text. Not system-prompt assembly; see [`crate::orchestrator::context`].

pub mod json_envelope;
pub mod post_tool_guidance;
pub mod protocol_schema;
