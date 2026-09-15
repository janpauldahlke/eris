//! Per-turn JSON-Schema subset compilation for OpenRouter, mirroring
//! [`super::llama_gbnf_subset::GbnfSubsetCache`] (same cache-key-by-sorted-tool-names strategy).
//! Both constraints derive from the same `Gatekeeper` tool schemas and the same offered-tool
//! list, so GBNF, envelope JSON-Schema, and native `tools[]` always offer the identical set.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::engine::structured::{
    EnvelopeToolEntry, OpenAiNativeTool, OpenAiSchema, build_envelope_json_schema, tool_args_schema,
};
use crate::executive::error::{FcpError, Result};
use crate::tools::Gatekeeper;

const CACHE_KEY_NO_TOOLS: &str = "__fcp_no_tools__";

struct CachedSubset {
    envelope: Arc<serde_json::Value>,
    native_tools: Arc<Vec<OpenAiNativeTool>>,
}

#[derive(Default)]
pub(crate) struct JsonSchemaSubsetCache {
    inner: Mutex<HashMap<String, CachedSubset>>,
}

impl JsonSchemaSubsetCache {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Returns cached or freshly built envelope schema for exactly `tool_names`
    /// (sorted internally for the key). Empty `tool_names` constrains `tool_calls` to `[]`.
    pub(crate) fn get_or_compile_subset(
        &self,
        gatekeeper: &Gatekeeper,
        tool_names: &[String],
    ) -> Result<Arc<serde_json::Value>> {
        Ok(self.get_or_compile(gatekeeper, tool_names)?.envelope)
    }

    /// Native OpenAI `tools[]` for the same offered names as [`Self::get_or_compile_subset`].
    /// Empty `tool_names` yields an empty list (do not attach `tools` on the wire).
    pub(crate) fn get_or_compile_native_tools(
        &self,
        gatekeeper: &Gatekeeper,
        tool_names: &[String],
    ) -> Result<Arc<Vec<OpenAiNativeTool>>> {
        Ok(self.get_or_compile(gatekeeper, tool_names)?.native_tools)
    }

    fn cache_key(tool_names: &[String]) -> (Vec<String>, String) {
        let mut sorted: Vec<String> = tool_names.to_vec();
        sorted.sort();
        let key = if sorted.is_empty() {
            CACHE_KEY_NO_TOOLS.to_string()
        } else {
            sorted.join("\x1e")
        };
        (sorted, key)
    }

    fn get_or_compile(
        &self,
        gatekeeper: &Gatekeeper,
        tool_names: &[String],
    ) -> Result<CachedSubset> {
        let (sorted, key) = Self::cache_key(tool_names);
        let mut guard = self.inner.lock().map_err(|_| {
            FcpError::EngineFault("JSON-Schema subset cache mutex poisoned".to_string())
        })?;

        if let Some(hit) = guard.get(&key) {
            return Ok(CachedSubset {
                envelope: Arc::clone(&hit.envelope),
                native_tools: Arc::clone(&hit.native_tools),
            });
        }

        let mut entries: Vec<EnvelopeToolEntry> = Vec::with_capacity(sorted.len());
        let mut native_tools: Vec<OpenAiNativeTool> = Vec::with_capacity(sorted.len());
        for name in &sorted {
            let args = gatekeeper
                .parameters_root_schema_for(name)
                .map(|schema| tool_args_schema(name, &schema))
                .unwrap_or_else(OpenAiSchema::empty_object);
            let description = gatekeeper.description_for(name).unwrap_or_default();
            native_tools.push(OpenAiNativeTool::function(
                name.clone(),
                description,
                args.to_value(),
            ));
            entries.push(EnvelopeToolEntry {
                name: name.clone(),
                args,
            });
        }

        let cached = CachedSubset {
            envelope: Arc::new(build_envelope_json_schema(&entries)),
            native_tools: Arc::new(native_tools),
        };
        guard.insert(
            key,
            CachedSubset {
                envelope: Arc::clone(&cached.envelope),
                native_tools: Arc::clone(&cached.native_tools),
            },
        );
        Ok(cached)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::traits::Tool;
    use async_trait::async_trait;
    use schemars::{JsonSchema, schema_for};
    use serde::Deserialize;

    #[derive(JsonSchema, Deserialize)]
    struct EmptyArgs {}

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct ReadArgs {
        relative_path: String,
    }

    struct HealthStub;

    #[async_trait]
    impl Tool for HealthStub {
        fn name(&self) -> &'static str {
            "system:health"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schema_for!(EmptyArgs)
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> crate::executive::error::Result<String> {
            Ok("{}".to_string())
        }
    }

    struct ReadStub;

    #[async_trait]
    impl Tool for ReadStub {
        fn name(&self) -> &'static str {
            "vault:read"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn parameters_schema(&self) -> schemars::schema::RootSchema {
            schema_for!(ReadArgs)
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> crate::executive::error::Result<String> {
            Ok("{}".to_string())
        }
    }

    fn gatekeeper() -> Gatekeeper {
        let mut gk = Gatekeeper::new();
        gk.register(std::sync::Arc::new(HealthStub));
        gk.register(std::sync::Arc::new(ReadStub));
        gk
    }

    #[test]
    fn subset_lists_only_offered_tool() {
        let gk = gatekeeper();
        let cache = JsonSchemaSubsetCache::new();
        let schema = cache
            .get_or_compile_subset(&gk, &["vault:read".into()])
            .expect("subset");
        let rendered = schema.to_string();
        assert!(rendered.contains("vault:read"));
        assert!(
            !rendered.contains("system:health"),
            "subset must not include a tool omitted from the offered set"
        );
        assert!(
            rendered.contains("relative_path"),
            "typed args survive lowering"
        );
    }

    #[test]
    fn cache_hits_return_same_arc_and_key_ignores_order() {
        let gk = gatekeeper();
        let cache = JsonSchemaSubsetCache::new();
        let a = cache
            .get_or_compile_subset(&gk, &["vault:read".into(), "system:health".into()])
            .expect("a");
        let b = cache
            .get_or_compile_subset(&gk, &["system:health".into(), "vault:read".into()])
            .expect("b");
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "sorted key must hit the cache"
        );
    }

    #[test]
    fn empty_offered_set_constrains_tool_calls() {
        let gk = gatekeeper();
        let cache = JsonSchemaSubsetCache::new();
        let schema = cache.get_or_compile_subset(&gk, &[]).expect("empty subset");
        assert_eq!(schema["properties"]["tool_calls"]["maxItems"], 0);
    }

    /// GBNF and JSON-Schema subsets are built from the same offered list, so the tool sets
    /// they expose must be identical.
    #[test]
    fn gbnf_and_json_schema_subsets_offer_identical_tools() {
        let gk = gatekeeper();
        let offered = vec!["system:health".to_string(), "vault:read".to_string()];

        let gbnf_cache = super::super::llama_gbnf_subset::GbnfSubsetCache::new();
        let gbnf = gbnf_cache
            .get_or_compile_subset(&gk, &offered)
            .expect("gbnf subset");
        let json_cache = JsonSchemaSubsetCache::new();
        let json = json_cache
            .get_or_compile_subset(&gk, &offered)
            .expect("json subset");
        let json_rendered = json.to_string();

        for name in &offered {
            assert!(gbnf.contains(name.as_str()), "GBNF missing {name}");
            assert!(
                json_rendered.contains(name.as_str()),
                "JSON schema missing {name}"
            );
        }
    }

    #[test]
    fn native_tools_match_offered_names_and_carry_strict_parameters() {
        let gk = gatekeeper();
        let cache = JsonSchemaSubsetCache::new();
        let offered = vec!["vault:read".to_string(), "system:health".to_string()];
        let tools = cache
            .get_or_compile_native_tools(&gk, &offered)
            .expect("native tools");
        let names: Vec<&str> = tools.iter().map(|t| t.function.name.as_str()).collect();
        assert_eq!(names, vec!["system:health", "vault:read"]);
        for t in tools.iter() {
            assert_eq!(t.kind, "function");
            assert!(t.function.strict);
            assert_eq!(t.function.parameters["type"], "object");
            assert_eq!(t.function.parameters["additionalProperties"], false);
        }
        let empty = cache
            .get_or_compile_native_tools(&gk, &[])
            .expect("empty native tools");
        assert!(empty.is_empty());
    }
}
