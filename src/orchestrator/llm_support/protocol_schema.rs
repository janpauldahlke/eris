use std::collections::HashSet;

use schemars::schema_for;
use serde_json::{Value, json};

use crate::orchestrator::state::LlmResponse;
use crate::tools::AllowedToolSchema;

fn base_llm_response_schema() -> Value {
    serde_json::to_value(schema_for!(LlmResponse)).unwrap_or_else(|_| {
        json!({
            "type": "object",
            "properties": {
                "thought": {"type": "string"},
                "status": {"type": ["string", "null"]},
                "message_to_user": {"type": ["string", "null"]},
                "tool_calls": {"type": "array"}
            },
            "required": ["thought", "tool_calls"],
            "additionalProperties": false
        })
    })
}

/// Build a strict per-turn schema for the assistant protocol response.
///
/// - Restricts `tool_calls[].name` to the active/authorized names for this turn.
/// - Couples each `name` with matching `args` schema (one branch per tool).
/// - When no tools are active, requires `tool_calls` to be an empty array.
pub fn build_llm_response_schema_for_tools(
    active_tools: &[String],
    allowed_tool_schemas: &[AllowedToolSchema],
) -> Value {
    let mut root = base_llm_response_schema();
    let active_set = active_tools.iter().cloned().collect::<HashSet<_>>();
    let mut selected = allowed_tool_schemas
        .iter()
        .filter(|tool| active_set.contains(&tool.name))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|a, b| a.name.cmp(&b.name));

    if let Some(props) = root.get_mut("properties").and_then(Value::as_object_mut) {
        // Require non-null, explicit status in constrained mode.
        props.insert(
            "status".to_string(),
            json!({
                "type": "string",
                "enum": ["Reflect", "Idle", "Task"]
            }),
        );
        let branches = selected
            .iter()
            .map(|tool| {
                json!({
                    "type": "object",
                    "properties": {
                        "name": {"const": tool.name},
                        "args": tool.schema
                    },
                    "required": ["name", "args"],
                    "additionalProperties": false
                })
            })
            .collect::<Vec<_>>();
        // `items: false` is valid in some JSON Schema drafts to forbid all elements, but
        // llama-server's schema→grammar path rejects the literal `false` ("Unrecognized schema: false").
        // An empty array is fully constrained by `maxItems: 0` alone.
        let tool_calls_schema = if branches.is_empty() {
            // Conversational/no-tools path: force a direct user reply.
            props.insert("status".to_string(), json!({"const": "Idle"}));
            props.insert(
                "message_to_user".to_string(),
                json!({
                    "type": "string",
                    "minLength": 1
                }),
            );
            json!({
                "type": "array",
                "maxItems": 0
            })
        } else {
            json!({
                "type": "array",
                "items": {
                    "oneOf": branches
                }
            })
        };
        props.insert("tool_calls".to_string(), tool_calls_schema);
    }
    if let Some(root_obj) = root.as_object_mut() {
        root_obj.insert("additionalProperties".to_string(), Value::Bool(false));
        if !selected.is_empty() {
            // In tool-enabled turns, require a concrete action (no zero-tool idle replies).
            root_obj.insert(
                "anyOf".to_string(),
                json!([
                    {
                        "type": "object",
                        "properties": {
                            "status": { "enum": ["Reflect", "Task"] },
                            "tool_calls": { "type": "array", "minItems": 1 }
                        },
                        "required": ["status", "tool_calls"]
                    }
                ]),
            );
        }
        let mut required = root_obj
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>();
        for key in ["status", "tool_calls"] {
            if !required.iter().any(|k| k == key) {
                required.push(key.to_string());
            }
        }
        // If no tools are active, we must require user-facing copy.
        if selected.is_empty() && !required.iter().any(|k| k == "message_to_user") {
            required.push("message_to_user".to_string());
        }
        root_obj.insert(
            "required".to_string(),
            Value::Array(required.into_iter().map(Value::String).collect()),
        );
    }
    root
}

#[cfg(test)]
mod tests {
    use super::build_llm_response_schema_for_tools;
    use jsonschema::JSONSchema;
    use serde_json::Value;
    use crate::tools::AllowedToolSchema;

    #[test]
    fn empty_tool_set_forces_empty_tool_calls() {
        let schema = build_llm_response_schema_for_tools(&[], &[]);
        let tool_calls = schema
            .get("properties")
            .and_then(|v| v.get("tool_calls"))
            .expect("tool_calls schema");
        assert_eq!(
            tool_calls.get("maxItems").and_then(|v| v.as_u64()),
            Some(0)
        );
        assert!(
            tool_calls.get("items").is_none(),
            "llama-server rejects items:false; use maxItems:0 only"
        );
        let status = schema
            .get("properties")
            .and_then(|v| v.get("status"))
            .expect("status schema");
        assert_eq!(status.get("const").and_then(|v| v.as_str()), Some("Idle"));
        let message = schema
            .get("properties")
            .and_then(|v| v.get("message_to_user"))
            .expect("message_to_user schema");
        assert_eq!(message.get("minLength").and_then(|v| v.as_u64()), Some(1));
    }

    #[test]
    fn active_tool_names_drive_one_of_branches() {
        let tools = vec![
            AllowedToolSchema {
                name: "clock:timer".to_string(),
                schema: serde_json::json!({
                    "type":"object",
                    "properties":{"minutes":{"type":"integer","minimum":0}},
                    "required":["minutes"],
                    "additionalProperties": false
                }),
            },
            AllowedToolSchema {
                name: "vault:read".to_string(),
                schema: serde_json::json!({
                    "type":"object",
                    "properties":{"path":{"type":"string"}},
                    "required":["path"],
                    "additionalProperties": false
                }),
            },
        ];
        let active = vec!["clock:timer".to_string()];
        let schema = build_llm_response_schema_for_tools(&active, &tools);
        let one_of_len = schema
            .get("properties")
            .and_then(|v| v.get("tool_calls"))
            .and_then(|v| v.get("items"))
            .and_then(|v| v.get("oneOf"))
            .and_then(Value::as_array)
            .map(|a| a.len());
        assert_eq!(one_of_len, Some(1));
    }

    #[test]
    fn schema_rejects_disallowed_tool_name() {
        let tools = vec![AllowedToolSchema {
            name: "clock:timer".to_string(),
            schema: serde_json::json!({
                "type":"object",
                "properties":{"minutes":{"type":"integer","minimum":0}},
                "required":["minutes"],
                "additionalProperties": false
            }),
        }];
        let active = vec!["clock:timer".to_string()];
        let schema = build_llm_response_schema_for_tools(&active, &tools);
        let compiled = JSONSchema::compile(&schema).expect("schema compiles");
        let bad = serde_json::json!({
            "thought": "x",
            "tool_calls": [
                {"name":"vault:write", "args":{"path":"a.md"}}
            ]
        });
        assert!(compiled.validate(&bad).is_err());
    }

    #[test]
    fn no_tools_schema_requires_message_to_user() {
        let schema = build_llm_response_schema_for_tools(&[], &[]);
        let compiled = JSONSchema::compile(&schema).expect("schema compiles");
        let bad = serde_json::json!({
            "status": "Idle",
            "tool_calls": []
        });
        assert!(compiled.validate(&bad).is_err());
        let good = serde_json::json!({
            "status": "Idle",
            "message_to_user": "Hello!",
            "tool_calls": []
        });
        assert!(compiled.validate(&good).is_ok());
    }

    #[test]
    fn tool_mode_schema_rejects_empty_action() {
        let tools = vec![AllowedToolSchema {
            name: "clock:timer".to_string(),
            schema: serde_json::json!({
                "type":"object",
                "properties":{"minutes":{"type":"integer","minimum":1}},
                "required":["minutes"],
                "additionalProperties": false
            }),
        }];
        let active = vec!["clock:timer".to_string()];
        let schema = build_llm_response_schema_for_tools(&active, &tools);
        let compiled = JSONSchema::compile(&schema).expect("schema compiles");
        let bad = serde_json::json!({
            "status": "Reflect",
            "tool_calls": [],
            "message_to_user": null
        });
        assert!(compiled.validate(&bad).is_err());
        let bad_idle_reply = serde_json::json!({
            "status": "Idle",
            "tool_calls": [],
            "message_to_user": "Done."
        });
        assert!(compiled.validate(&bad_idle_reply).is_err());
        let good_action = serde_json::json!({
            "status": "Task",
            "tool_calls": [
                {"name":"clock:timer","args":{"minutes":10}}
            ]
        });
        assert!(compiled.validate(&good_action).is_ok());
    }
}
