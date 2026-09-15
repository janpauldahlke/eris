//! Full FCP-envelope JSON Schema for OpenAI strict structured outputs. Mirrors
//! [`crate::engine::grammar::compile_fcp_envelope_grammar_dynamic`]: the fixed envelope shape
//! (`thought` / `status` / `message_to_user` / `tool_calls`) with `tool_calls` items as a
//! discriminated union (`anyOf` keyed on `name`) over the offered tools. An empty offered set
//! constrains `tool_calls` to `[]`.

use serde_json::{Value, json};

use super::schema_to_openai::OpenAiSchema;

/// One offered tool: name plus its lowered args schema (already fallback-resolved).
pub struct EnvelopeToolEntry {
    pub name: String,
    pub args: OpenAiSchema,
}

fn tool_call_item(entry: &EnvelopeToolEntry) -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "enum": [entry.name] },
            "args": entry.args.to_value(),
        },
        "required": ["name", "args"],
        "additionalProperties": false,
    })
}

/// Build the strict envelope schema for exactly the offered tools.
#[must_use]
pub fn build_envelope_json_schema(tools: &[EnvelopeToolEntry]) -> Value {
    let tool_calls = match tools {
        [] => json!({
            "type": "array",
            "items": { "type": "object", "properties": {}, "required": [], "additionalProperties": false },
            "maxItems": 0,
        }),
        [single] => json!({
            "type": "array",
            "items": tool_call_item(single),
        }),
        many => {
            let alternatives: Vec<Value> = many.iter().map(tool_call_item).collect();
            json!({
                "type": "array",
                "items": { "anyOf": alternatives },
            })
        }
    };

    json!({
        "type": "object",
        "properties": {
            "thought": { "type": "string" },
            "status": { "type": "string", "enum": ["Task", "Reflect", "Idle", "Process"] },
            "message_to_user": { "anyOf": [ { "type": "string" }, { "type": "null" } ] },
            "tool_calls": tool_calls,
        },
        "required": ["thought", "status", "message_to_user", "tool_calls"],
        "additionalProperties": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed_entry(name: &str) -> EnvelopeToolEntry {
        EnvelopeToolEntry {
            name: name.into(),
            args: OpenAiSchema::Object {
                properties: vec![(
                    "relative_path".into(),
                    OpenAiSchema::String { enum_values: None },
                )],
            },
        }
    }

    fn fallback_entry(name: &str) -> EnvelopeToolEntry {
        EnvelopeToolEntry {
            name: name.into(),
            args: OpenAiSchema::empty_object(),
        }
    }

    #[test]
    fn envelope_shape_is_fixed_and_closed() {
        let v = build_envelope_json_schema(&[]);
        assert_eq!(v["type"], "object");
        assert_eq!(v["additionalProperties"], false);
        assert_eq!(
            v["required"],
            serde_json::json!(["thought", "status", "message_to_user", "tool_calls"])
        );
        assert_eq!(
            v["properties"]["status"]["enum"],
            serde_json::json!(["Task", "Reflect", "Idle", "Process"])
        );
        let msg_union = v["properties"]["message_to_user"]["anyOf"]
            .as_array()
            .expect("anyOf");
        assert!(msg_union.iter().any(|s| s["type"] == "null"));
    }

    #[test]
    fn empty_offered_set_constrains_tool_calls_to_empty_array() {
        let v = build_envelope_json_schema(&[]);
        assert_eq!(v["properties"]["tool_calls"]["maxItems"], 0);
    }

    #[test]
    fn single_tool_items_is_direct_object_with_name_enum() {
        let v = build_envelope_json_schema(&[typed_entry("vault:read")]);
        let items = &v["properties"]["tool_calls"]["items"];
        assert_eq!(
            items["properties"]["name"]["enum"],
            serde_json::json!(["vault:read"])
        );
        assert_eq!(
            items["properties"]["args"]["properties"]["relative_path"]["type"],
            "string"
        );
        assert_eq!(items["additionalProperties"], false);
    }

    #[test]
    fn multiple_tools_form_discriminated_union() {
        let v = build_envelope_json_schema(&[
            typed_entry("vault:read"),
            fallback_entry("memory:stage"),
            typed_entry("web:fetch"),
        ]);
        let alts = v["properties"]["tool_calls"]["items"]["anyOf"]
            .as_array()
            .expect("anyOf union");
        assert_eq!(alts.len(), 3);
        let names: Vec<&str> = alts
            .iter()
            .filter_map(|a| a["properties"]["name"]["enum"][0].as_str())
            .collect();
        assert_eq!(names, vec!["vault:read", "memory:stage", "web:fetch"]);
        // Fallback tool gets the closed empty-object args.
        assert_eq!(alts[1]["properties"]["args"]["additionalProperties"], false);
    }
}
