//! Internal HTTP client for templated API profiles ([`crate::config::ApiProfile`]). Not an LLM `Tool`.

mod client;
mod template;

pub use client::ApiHttpClient;
