use super::schema_to_gbnf::GbnfRule;
use super::tool_names::build_tool_name_enum;

/// Static GBNF skeleton — shared between static-args and per-tool-args modes.
const STATIC_GRAMMAR: &str = r#"root ::= "{" ws thought-kv "," ws status-kv "," ws message-kv "," ws toolcalls-kv ws "}"

ws ::= [ \t\n]*

thought-kv ::= "\"thought\"" ws ":" ws json-string
status-kv  ::= "\"status\"" ws ":" ws status-enum
message-kv ::= "\"message_to_user\"" ws ":" ws (json-string | "null")
toolcalls-kv ::= "\"tool_calls\"" ws ":" ws "[" ws tool-call-list ws "]"

status-enum ::= "\"Task\"" | "\"Reflect\"" | "\"Idle\"" | "\"Process\""

json-object ::= "{" ws json-members? ws "}"
json-members ::= json-pair ("," ws json-pair)*
json-pair ::= json-string ws ":" ws json-value
json-value ::= json-string | json-number | json-object | json-array | "true" | "false" | "null"
json-array ::= "[" ws json-elements? ws "]"
json-elements ::= json-value ("," ws json-value)*
json-string ::= "\"" json-char* "\""
json-char ::= [^"\\] | "\\" json-escape
json-escape ::= ["\\nrtbf/] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F]
json-number ::= "-"? ("0" | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?
"#;

/// GBNF rules for the legacy static-args path (Phase 4 fallback).
const TOOL_CALL_RULES_TEMPLATE: &str = r#"tool-call-list ::= "" | tool-call ("," ws tool-call)*
tool-call ::= "{" ws "\"name\"" ws ":" ws tool-name-enum ws "," ws "\"args\"" ws ":" ws json-object ws "}"
"#;

/// Per-tool dynamic-args version: tool-call dispatches into `tool-with-args`.
const TOOL_CALL_RULES_DYNAMIC: &str = r#"tool-call-list ::= "" | tool-call ("," ws tool-call)*
tool-call ::= "{" ws "\"name\"" ws ":" ws tool-with-args ws "}"
"#;

/// GBNF rule for when no tools are registered — only empty arrays are valid.
const NO_TOOL_RULES: &str = "tool-call-list ::= \"\"\n";

/// Build a complete GBNF grammar string for the FCP protocol envelope (Phase 4 — static args).
///
/// `tool_names` is the set of registered tool names (e.g., `["vault:read", "vault:write", ...]`).
/// Returns a ready-to-use GBNF string that constrains LLM output to valid `LlmResponse` JSON.
pub fn compile_fcp_envelope_grammar(tool_names: &[String]) -> String {
    let mut grammar = String::with_capacity(2048);
    grammar.push_str(STATIC_GRAMMAR);
    grammar.push('\n');

    if tool_names.is_empty() {
        grammar.push_str(NO_TOOL_RULES);
    } else {
        grammar.push_str(TOOL_CALL_RULES_TEMPLATE);
        let enum_body = build_tool_name_enum(tool_names);
        grammar.push_str(&format!("tool-name-enum ::= {enum_body}\n"));
    }

    grammar
}

/// Per-tool entry for [`compile_fcp_envelope_grammar_dynamic`].
///
/// `name` is the tool name (e.g. `"vault:read"`).
/// `per_tool_rules` is `Some(rules)` when the schema compiled successfully,
/// where `rules[0]` is the main args rule for this tool and the rest are helpers.
/// `None` means this tool falls back to `json-object` for its args.
pub struct ToolGrammarEntry {
    pub name: String,
    pub per_tool_rules: Option<Vec<GbnfRule>>,
}

/// Build a complete GBNF grammar string with **per-tool arg rules** (Phase 7).
///
/// Each tool's `name` field is coupled with its specific args shape in the
/// `tool-with-args` alternation. Tools that couldn't be compiled fall back to
/// `json-object`.
pub fn compile_fcp_envelope_grammar_dynamic(tools: &[ToolGrammarEntry]) -> String {
    let mut grammar = String::with_capacity(4096);
    grammar.push_str(STATIC_GRAMMAR);
    grammar.push('\n');

    if tools.is_empty() {
        grammar.push_str(NO_TOOL_RULES);
        return grammar;
    }

    grammar.push_str(TOOL_CALL_RULES_DYNAMIC);

    let mut alternation_parts: Vec<String> = Vec::with_capacity(tools.len());

    for entry in tools {
        let quoted_name = format!("\"\\\"{}\\\"\"", entry.name);
        match &entry.per_tool_rules {
            Some(rules) if !rules.is_empty() => {
                let main_rule_name = &rules[0].0;
                alternation_parts.push(format!(
                    "{quoted_name} ws \",\" ws \"\\\"args\\\"\" ws \":\" ws {main_rule_name}"
                ));
                for (rule_name, rule_body) in rules {
                    grammar.push_str(&format!("{rule_name} ::= {rule_body}\n"));
                }
            }
            _ => {
                alternation_parts.push(format!(
                    "{quoted_name} ws \",\" ws \"\\\"args\\\"\" ws \":\" ws json-object"
                ));
            }
        }
    }

    let alternation = alternation_parts.join("\n  | ");
    grammar.push_str(&format!("tool-with-args ::=\n    {alternation}\n"));

    grammar
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tool_names() -> Vec<String> {
        vec![
            "vault:read".into(),
            "vault:write".into(),
            "memory:stage".into(),
            "web:fetch".into(),
        ]
    }

    #[test]
    fn compile_with_tools_produces_valid_gbnf() {
        let grammar = compile_fcp_envelope_grammar(&sample_tool_names());
        assert!(!grammar.is_empty());
        assert!(grammar.contains("root ::="));
        assert!(grammar.contains("tool-name-enum ::="));
        for name in &sample_tool_names() {
            assert!(grammar.contains(name), "grammar should contain {name}");
        }
    }

    #[test]
    fn compile_with_empty_tools() {
        let grammar = compile_fcp_envelope_grammar(&[]);
        assert!(grammar.contains("root ::="));
        assert!(!grammar.contains("tool-name-enum"));
        assert!(grammar.contains("tool-call-list ::= \"\""));
    }

    #[test]
    fn grammar_contains_all_status_values() {
        let grammar = compile_fcp_envelope_grammar(&sample_tool_names());
        for status in &["Task", "Reflect", "Idle", "Process"] {
            assert!(
                grammar.contains(status),
                "grammar should contain status {status}"
            );
        }
    }

    /// Validate that representative LLM JSON outputs parse as `LlmResponse`.
    /// This indirectly validates the grammar's design: the shapes we allow in the grammar
    /// are the shapes serde can parse.
    mod shape_validation {
        use serde::Deserialize;

        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct LlmResponseShape {
            thought: String,
            status: String,
            message_to_user: Option<String>,
            tool_calls: Vec<ToolCallShape>,
        }

        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct ToolCallShape {
            name: String,
            args: serde_json::Value,
        }

        fn parse(json: &str) -> LlmResponseShape {
            serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("failed to parse: {e}\njson: {json}"))
        }

        #[test]
        fn idle_response_matches_grammar_shape() {
            let r = parse(
                r#"{"thought":"thinking","status":"Idle","message_to_user":"hi","tool_calls":[]}"#,
            );
            assert_eq!(r.status, "Idle");
            assert_eq!(r.message_to_user.as_deref(), Some("hi"));
            assert!(r.tool_calls.is_empty());
        }

        #[test]
        fn task_with_tool_call_matches() {
            let r = parse(
                r#"{"thought":"need file","status":"Reflect","message_to_user":null,"tool_calls":[{"name":"vault:read","args":{"path":"notes/today.md"}}]}"#,
            );
            assert_eq!(r.tool_calls.len(), 1);
            assert_eq!(r.tool_calls[0].name, "vault:read");
        }

        #[test]
        fn task_with_numeric_args_matches() {
            let r = parse(
                r#"{"thought":"set alarm","status":"Reflect","message_to_user":null,"tool_calls":[{"name":"alarm:set","args":{"minutes":30}}]}"#,
            );
            assert_eq!(r.tool_calls[0].args["minutes"], 30);
        }

        #[test]
        fn task_with_nested_args_matches() {
            let r = parse(
                r#"{"thought":"complex","status":"Reflect","message_to_user":null,"tool_calls":[{"name":"web:fetch","args":{"url":"https://example.com","options":{"timeout":5,"headers":{"Accept":"text/html"}}}}]}"#,
            );
            assert!(r.tool_calls[0].args["options"]["headers"]["Accept"]
                .as_str()
                .is_some());
        }

        #[test]
        fn task_with_multiple_tool_calls_matches() {
            let r = parse(
                r#"{"thought":"multi","status":"Reflect","message_to_user":null,"tool_calls":[{"name":"vault:read","args":{"path":"a.md"}},{"name":"vault:write","args":{"path":"b.md","content":"hello"}}]}"#,
            );
            assert_eq!(r.tool_calls.len(), 2);
        }

        #[test]
        fn null_message_matches() {
            let r = parse(
                r#"{"thought":"x","status":"Task","message_to_user":null,"tool_calls":[]}"#,
            );
            assert!(r.message_to_user.is_none());
        }

        #[test]
        fn thought_with_special_chars_matches() {
            let r = parse(
                r#"{"thought":"line1\nline2\t\"quoted\" back\\slash","status":"Idle","message_to_user":"ok","tool_calls":[]}"#,
            );
            assert!(r.thought.contains("line1"));
        }

        #[test]
        fn process_status_alias_matches() {
            let r = parse(
                r#"{"thought":"plan","status":"Process","message_to_user":null,"tool_calls":[]}"#,
            );
            assert_eq!(r.status, "Process");
        }
    }

    mod dynamic_grammar {
        use super::*;

        fn typed_entry(name: &str, rule_name: &str, rule_body: &str) -> ToolGrammarEntry {
            ToolGrammarEntry {
                name: name.into(),
                per_tool_rules: Some(vec![(rule_name.into(), rule_body.into())]),
            }
        }

        fn fallback_entry(name: &str) -> ToolGrammarEntry {
            ToolGrammarEntry {
                name: name.into(),
                per_tool_rules: None,
            }
        }

        #[test]
        fn dynamic_empty_tools() {
            let grammar = compile_fcp_envelope_grammar_dynamic(&[]);
            assert!(grammar.contains("root ::="));
            assert!(grammar.contains("tool-call-list ::= \"\""));
            assert!(!grammar.contains("tool-with-args"));
        }

        #[test]
        fn dynamic_with_typed_and_fallback() {
            let entries = vec![
                typed_entry(
                    "vault:read",
                    "vault-read-args",
                    r#""{" ws "\"relative_path\"" ws ":" ws json-string ws "}""#,
                ),
                fallback_entry("memory:stage"),
            ];
            let grammar = compile_fcp_envelope_grammar_dynamic(&entries);
            assert!(grammar.contains("tool-with-args"));
            assert!(grammar.contains("vault:read"));
            assert!(grammar.contains("vault-read-args"));
            assert!(grammar.contains("memory:stage"));
            assert!(
                grammar.contains("json-object"),
                "fallback tool should use json-object"
            );
        }

        #[test]
        fn dynamic_grammar_contains_all_status_values() {
            let entries = vec![typed_entry(
                "test:tool",
                "test-tool-args",
                "\"{\" ws \"}\"",
            )];
            let grammar = compile_fcp_envelope_grammar_dynamic(&entries);
            for status in &["Task", "Reflect", "Idle", "Process"] {
                assert!(grammar.contains(status));
            }
        }

        #[test]
        fn dynamic_grammar_emits_extra_rules() {
            let entries = vec![ToolGrammarEntry {
                name: "test:array".into(),
                per_tool_rules: Some(vec![
                    (
                        "test-array-args".into(),
                        "\"{\" ws \"\\\"tags\\\"\" ws \":\" ws \"[\" ws (test-array-args-tags-list)? ws \"]\" ws \"}\"".into(),
                    ),
                    (
                        "test-array-args-tags-list".into(),
                        "json-string (\",\" ws json-string)*".into(),
                    ),
                ]),
            }];
            let grammar = compile_fcp_envelope_grammar_dynamic(&entries);
            assert!(grammar.contains("test-array-args-tags-list ::="));
            assert!(grammar.contains("json-string (\",\" ws json-string)*"));
        }
    }
}
