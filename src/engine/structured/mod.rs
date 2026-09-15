//! Structured-output schemas for hosted OpenAI-compatible backends (OpenRouter).
//!
//! Parallel to [`crate::engine::grammar`] (GBNF for llama.cpp): both derive their per-turn
//! constraint from the same `Gatekeeper` tool JSON Schemas and the same offered-tool set, so
//! prompt, constraint, and validation cannot drift apart.

pub mod envelope_schema;
pub mod schema_to_openai;

pub use envelope_schema::{EnvelopeToolEntry, build_envelope_json_schema};
pub use schema_to_openai::{OpenAiSchema, lower_root_schema, tool_args_schema};
