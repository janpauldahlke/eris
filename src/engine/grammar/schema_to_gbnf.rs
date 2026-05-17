use schemars::schema::{
    ArrayValidation, InstanceType, ObjectValidation, RootSchema, Schema, SchemaObject,
    SingleOrVec,
};

/// A compiled per-tool GBNF rule: `(rule_name, rule_body)`.
/// `rule_name` is e.g. `vault-read-args`, `rule_body` is the RHS of `::=`.
pub type GbnfRule = (String, String);

/// Convert a tool name to a GBNF-safe rule name.
/// `vault:read` → `vault-read-args`, `memory:stage` → `memory-stage-args`.
fn tool_name_to_rule_name(tool_name: &str) -> String {
    let sanitized = tool_name.replace(':', "-").replace('_', "-");
    format!("{sanitized}-args")
}

/// Compile a per-tool arg rule from its JSON Schema.
///
/// Returns `Some((rule_name, accumulated_rules))` where `accumulated_rules` is
/// all GBNF rules needed (the main args rule plus any helper rules like array
/// element lists). Returns `None` if the schema is too complex for grammar
/// coverage (caller should fall back to `json-object`).
pub fn schema_to_gbnf_rule(
    tool_name: &str,
    schema: &RootSchema,
) -> Option<(String, Vec<GbnfRule>)> {
    let rule_name = tool_name_to_rule_name(tool_name);
    let mut ctx = CompileCtx {
        definitions: &schema.definitions,
        extra_rules: Vec::new(),
        depth: 0,
    };

    let body = compile_schema_object(&schema.schema, &rule_name, &mut ctx)?;
    let mut rules = vec![(rule_name.clone(), body)];
    rules.extend(ctx.extra_rules);
    Some((rule_name, rules))
}

const MAX_DEPTH: u8 = 2;

struct CompileCtx<'a> {
    definitions: &'a schemars::Map<String, Schema>,
    extra_rules: Vec<GbnfRule>,
    depth: u8,
}

/// Compile a `SchemaObject` into a GBNF expression (the RHS of a rule or inline).
/// Returns `None` if unsupported.
fn compile_schema_object(
    schema: &SchemaObject,
    parent_rule_name: &str,
    ctx: &mut CompileCtx<'_>,
) -> Option<String> {
    if schema.subschemas.is_some() {
        let subs = schema.subschemas.as_ref()?;
        if subs.one_of.is_some() || subs.any_of.is_some() || subs.all_of.is_some() {
            tracing::warn!(
                rule = parent_rule_name,
                "schema_to_gbnf: unsupported subschema (oneOf/anyOf/allOf), falling back"
            );
            return None;
        }
    }

    if let Some(ref reference) = schema.reference {
        return resolve_ref(reference, parent_rule_name, ctx);
    }

    let instance_type = match &schema.instance_type {
        Some(SingleOrVec::Single(t)) => Some(**t),
        Some(SingleOrVec::Vec(types)) => {
            if types.len() == 2 && types.contains(&InstanceType::Null) {
                let non_null = types.iter().find(|t| **t != InstanceType::Null)?;
                return compile_nullable_type(*non_null, schema, parent_rule_name, ctx);
            }
            tracing::warn!(
                rule = parent_rule_name,
                "schema_to_gbnf: multi-type (non-nullable) not supported"
            );
            return None;
        }
        None => None,
    };

    match instance_type {
        Some(InstanceType::Object) => {
            compile_object(schema.object.as_deref(), parent_rule_name, ctx)
        }
        Some(InstanceType::String) => Some(compile_string(schema)),
        Some(InstanceType::Integer) | Some(InstanceType::Number) => Some("json-number".into()),
        Some(InstanceType::Boolean) => Some(r#"("true" | "false")"#.into()),
        Some(InstanceType::Array) => {
            compile_array(schema.array.as_deref(), parent_rule_name, ctx)
        }
        Some(InstanceType::Null) => Some("\"null\"".into()),
        None => {
            if schema.enum_values.is_some() {
                Some(compile_string(schema))
            } else if schema.object.is_some() {
                compile_object(schema.object.as_deref(), parent_rule_name, ctx)
            } else {
                tracing::warn!(
                    rule = parent_rule_name,
                    "schema_to_gbnf: no instance_type and no recognizable shape"
                );
                None
            }
        }
    }
}

fn resolve_ref(
    reference: &str,
    parent_rule_name: &str,
    ctx: &mut CompileCtx<'_>,
) -> Option<String> {
    let def_name = reference.strip_prefix("#/definitions/")?;
    let definition = ctx.definitions.get(def_name)?;
    match definition {
        Schema::Object(obj) => compile_schema_object(obj, parent_rule_name, ctx),
        Schema::Bool(_) => None,
    }
}

fn compile_nullable_type(
    inner: InstanceType,
    schema: &SchemaObject,
    parent_rule_name: &str,
    ctx: &mut CompileCtx<'_>,
) -> Option<String> {
    let mut non_null_schema = schema.clone();
    non_null_schema.instance_type = Some(SingleOrVec::Single(Box::new(inner)));
    let inner_expr = compile_schema_object(&non_null_schema, parent_rule_name, ctx)?;
    Some(format!("({inner_expr} | \"null\")"))
}

fn compile_string(schema: &SchemaObject) -> String {
    if let Some(ref enum_values) = schema.enum_values {
        let alts: Vec<String> = enum_values
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| format!("\"\\\"{}\\\"\"", s))
            .collect();
        if !alts.is_empty() {
            return format!("({})", alts.join(" | "));
        }
    }
    "json-string".into()
}

fn compile_object(
    validation: Option<&ObjectValidation>,
    parent_rule_name: &str,
    ctx: &mut CompileCtx<'_>,
) -> Option<String> {
    let validation = match validation {
        Some(v) => v,
        None => return Some("\"{\" ws \"}\"".into()),
    };

    if validation.additional_properties.as_deref().is_some_and(|s| !matches!(s, Schema::Bool(false))) {
        if !validation.properties.is_empty() {
            // has both fixed properties and additional — we can still handle the fixed ones
            // but this is a complex edge case; fall back for safety
        }
        if validation.properties.is_empty() {
            tracing::warn!(
                rule = parent_rule_name,
                "schema_to_gbnf: free-form additionalProperties, falling back"
            );
            return None;
        }
    }

    if ctx.depth >= MAX_DEPTH {
        tracing::warn!(
            rule = parent_rule_name,
            depth = ctx.depth,
            "schema_to_gbnf: max nesting depth exceeded, falling back"
        );
        return None;
    }
    ctx.depth += 1;

    let required_set: std::collections::HashSet<&str> = validation
        .required
        .iter()
        .map(|s| s.as_str())
        .collect();

    let mut sorted_keys: Vec<&String> = validation.properties.keys().collect();
    sorted_keys.sort();

    struct FieldInfo {
        key: String,
        expr: String,
        required: bool,
    }

    let mut fields: Vec<FieldInfo> = Vec::new();
    for key in sorted_keys {
        let prop_schema = match validation.properties.get(key) {
            Some(Schema::Object(obj)) => obj,
            Some(Schema::Bool(true)) => {
                fields.push(FieldInfo {
                    key: key.clone(),
                    expr: "json-value".into(),
                    required: required_set.contains(key.as_str()),
                });
                continue;
            }
            _ => continue,
        };
        let sub_rule_name = format!("{parent_rule_name}-{}", key.replace('_', "-"));
        match compile_schema_object(prop_schema, &sub_rule_name, ctx) {
            Some(expr) => {
                fields.push(FieldInfo {
                    key: key.clone(),
                    expr,
                    required: required_set.contains(key.as_str()),
                });
            }
            None => {
                fields.push(FieldInfo {
                    key: key.clone(),
                    expr: "json-value".into(),
                    required: required_set.contains(key.as_str()),
                });
            }
        }
    }

    ctx.depth -= 1;

    if fields.is_empty() {
        return Some("\"{\" ws \"}\"".into());
    }

    // Build the object rule with the trailing-comma-safe approach:
    // Required fields get unconditional emission, optional fields get `(...)?` with leading comma.
    // Sort: required first (alphabetically), then optional (alphabetically).
    let mut required_fields: Vec<&FieldInfo> = fields.iter().filter(|f| f.required).collect();
    let mut optional_fields: Vec<&FieldInfo> = fields.iter().filter(|f| !f.required).collect();
    required_fields.sort_by_key(|f| &f.key);
    optional_fields.sort_by_key(|f| &f.key);

    // Strategy: emit all keys in sorted order (required first, optional last).
    // The first key has no leading comma; all subsequent keys have a leading comma.
    // Optional keys wrap the comma+key-value in (...)?
    let all_fields: Vec<&FieldInfo> = required_fields
        .iter()
        .chain(optional_fields.iter())
        .copied()
        .collect();

    let mut parts = Vec::new();
    for (i, field) in all_fields.iter().enumerate() {
        let kv = format!(
            "\"\\\"{}\\\"\" ws \":\" ws {}",
            field.key, field.expr
        );
        if i == 0 {
            if field.required {
                parts.push(kv);
            } else {
                // First field is optional — special case: no leading comma
                parts.push(format!("({kv} )?"));
            }
        } else if field.required {
            // Required field with leading comma
            // But we need to handle the case where all preceding fields were optional
            // and none were emitted. We check: is there at least one required field before us?
            let has_required_before = all_fields[..i].iter().any(|f| f.required);
            if has_required_before {
                parts.push(format!("ws \",\" ws {kv}"));
            } else {
                // All before us are optional — we need conditional comma
                // If any optional field before was present, there's already content
                // We use a different approach: this required field always appears,
                // but the comma depends on whether anything came before.
                // Since this is complex in pure GBNF, we use the simpler approach:
                // required fields before optional fields, always.
                // This should not happen with our sorting (required first, optional last).
                // But as a safety fallback:
                parts.push(format!("ws \",\" ws {kv}"));
            }
        } else {
            // Optional field with leading comma
            parts.push(format!("(ws \",\" ws {kv} )?"));
        }
    }

    let body = format!("\"{{\" ws {} ws \"}}\"", parts.join(" "));
    Some(body)
}

fn compile_array(
    validation: Option<&ArrayValidation>,
    parent_rule_name: &str,
    ctx: &mut CompileCtx<'_>,
) -> Option<String> {
    let items_expr = match validation.and_then(|v| v.items.as_ref()) {
        Some(SingleOrVec::Single(item_schema)) => match item_schema.as_ref() {
            Schema::Object(obj) => {
                let item_rule = format!("{parent_rule_name}-item");
                compile_schema_object(obj, &item_rule, ctx).unwrap_or("json-value".into())
            }
            Schema::Bool(true) => "json-value".into(),
            Schema::Bool(false) => return Some("\"[\" ws \"]\"".into()),
        },
        Some(SingleOrVec::Vec(_)) => "json-value".into(),
        None => "json-value".into(),
    };

    let list_rule_name = format!("{parent_rule_name}-list");
    let list_body = format!("{items_expr} (\",\" ws {items_expr})*");
    ctx.extra_rules
        .push((list_rule_name.clone(), list_body));

    Some(format!(
        "\"[\" ws ({list_rule_name})? ws \"]\"",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[allow(dead_code)]
    fn compile_for<T: JsonSchema>(tool_name: &str) -> Option<(String, Vec<GbnfRule>)> {
        let schema = schemars::schema_for!(T);
        schema_to_gbnf_rule(tool_name, &schema)
    }

    fn get_main_rule(result: &Option<(String, Vec<GbnfRule>)>) -> &str {
        let (_, rules) = result.as_ref().unwrap();
        &rules[0].1
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct SimpleRequired {
        relative_path: String,
    }

    #[test]
    fn simple_required_string_field() {
        let result = compile_for::<SimpleRequired>("vault:read");
        let (name, rules) = result.as_ref().unwrap();
        assert_eq!(name, "vault-read-args");
        assert!(rules[0].1.contains("relative_path"));
        assert!(rules[0].1.contains("json-string"));
        assert!(!rules[0].1.contains('?'));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct ThreeRequired {
        zebra: String,
        alpha: String,
        middle: String,
    }

    #[test]
    fn multiple_required_fields_sorted() {
        let result = compile_for::<ThreeRequired>("test:sorted");
        let body = get_main_rule(&result);
        let alpha_pos = body.find("alpha").unwrap();
        let middle_pos = body.find("middle").unwrap();
        let zebra_pos = body.find("zebra").unwrap();
        assert!(alpha_pos < middle_pos);
        assert!(middle_pos < zebra_pos);
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    #[serde(rename_all = "lowercase")]
    enum TestMode {
        Overwrite,
        Append,
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct WithEnum {
        mode: TestMode,
    }

    #[test]
    fn enum_string_field() {
        let result = compile_for::<WithEnum>("test:enum");
        let body = get_main_rule(&result);
        assert!(body.contains("overwrite"));
        assert!(body.contains("append"));
        assert!(body.contains('|'));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct WithInteger {
        minutes: u32,
    }

    #[test]
    fn integer_field() {
        let result = compile_for::<WithInteger>("test:int");
        let body = get_main_rule(&result);
        assert!(body.contains("json-number"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct WithBool {
        permanent: bool,
    }

    #[test]
    fn boolean_field() {
        let result = compile_for::<WithBool>("test:bool");
        let body = get_main_rule(&result);
        assert!(body.contains("\"true\""));
        assert!(body.contains("\"false\""));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct WithArray {
        tags: Vec<String>,
    }

    #[test]
    fn array_of_strings() {
        let result = compile_for::<WithArray>("test:array");
        let (_, rules) = result.as_ref().unwrap();
        let body = &rules[0].1;
        assert!(body.contains("["));
        assert!(body.contains("]"));
        assert!(rules.len() >= 2, "should have the main rule + array list rule");
        let list_rule = &rules[1];
        assert!(list_rule.0.contains("list"));
        assert!(list_rule.1.contains("json-string"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct WithOptional {
        required_field: String,
        optional_field: Option<String>,
    }

    #[test]
    fn optional_field_syntax() {
        let result = compile_for::<WithOptional>("test:opt");
        let body = get_main_rule(&result);
        assert!(body.contains("required_field"));
        assert!(body.contains("optional_field"));
        assert!(body.contains(")?"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct MixedFields {
        name: String,
        count: u32,
        label: Option<String>,
    }

    #[test]
    fn mixed_required_optional() {
        let result = compile_for::<MixedFields>("test:mixed");
        let body = get_main_rule(&result);
        let count_pos = body.find("count").unwrap();
        let name_pos = body.find("name").unwrap();
        let label_pos = body.find("label").unwrap();
        assert!(count_pos < name_pos, "count before name (both required, alphabetical)");
        assert!(name_pos < label_pos, "label (optional) after all required");
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct AllOptional {
        alpha: Option<String>,
        beta: Option<u32>,
    }

    #[test]
    fn all_optional_fields() {
        let result = compile_for::<AllOptional>("test:allopt");
        let body = get_main_rule(&result);
        assert!(body.contains(")?"));
        assert!(body.contains("{"));
        assert!(body.contains("}"));
        assert!(!body.contains("\",\" ws \",\""));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct EmptyArgs {}

    #[test]
    fn empty_args() {
        let result = compile_for::<EmptyArgs>("system:health");
        let body = get_main_rule(&result);
        assert_eq!(body, "\"{\" ws \"}\"");
    }

    #[test]
    fn unsupported_schema_returns_none() {
        use schemars::schema::SubschemaValidation;
        let mut root = schemars::schema_for!(EmptyArgs);
        root.schema.subschemas = Some(Box::new(SubschemaValidation {
            one_of: Some(vec![Schema::Bool(true)]),
            ..Default::default()
        }));
        let result = schema_to_gbnf_rule("test:unsupported", &root);
        assert!(result.is_none());
    }

    #[test]
    fn test_tool_name_to_rule_name() {
        assert_eq!(tool_name_to_rule_name("vault:read"), "vault-read-args");
        assert_eq!(tool_name_to_rule_name("memory:stage"), "memory-stage-args");
        assert_eq!(tool_name_to_rule_name("web:find"), "web-find-args");
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    #[serde(rename_all = "lowercase")]
    enum RealWriteMode {
        Overwrite,
        Append,
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct RealVaultWriteArgs {
        relative_path: String,
        content: String,
        mode: RealWriteMode,
    }

    #[test]
    fn vault_write_real_schema() {
        let result = compile_for::<RealVaultWriteArgs>("vault:write");
        assert!(result.is_some(), "vault:write should compile");
        let (name, rules) = result.as_ref().unwrap();
        assert_eq!(name, "vault-write-args");
        let body = &rules[0].1;
        assert!(body.contains("content"));
        assert!(body.contains("mode"));
        assert!(body.contains("relative_path"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct RealClockTimerArgs {
        minutes: u32,
        label: String,
    }

    #[test]
    fn clock_timer_real_schema() {
        let result = compile_for::<RealClockTimerArgs>("clock:timer");
        assert!(result.is_some());
        let (_, rules) = result.as_ref().unwrap();
        let body = &rules[0].1;
        assert!(body.contains("label"));
        assert!(body.contains("minutes"));
        assert!(body.contains("json-number"));
        assert!(body.contains("json-string"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct RealAgendaRemindAtArgs {
        task_id: Option<String>,
        description: Option<String>,
        minutes: Option<u32>,
        hour: Option<u8>,
        minute: Option<u8>,
    }

    #[test]
    fn agenda_remind_at_all_optional() {
        let result = compile_for::<RealAgendaRemindAtArgs>("agenda:remind_at");
        assert!(result.is_some(), "agenda:remind_at should compile");
        let (_, rules) = result.as_ref().unwrap();
        let body = &rules[0].1;
        assert!(body.contains(")?"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct NullableFieldArgs {
        name: String,
        notes: Option<String>,
    }

    #[test]
    fn nullable_field_handled() {
        let result = compile_for::<NullableFieldArgs>("test:nullable");
        assert!(result.is_some());
        let body = get_main_rule(&result);
        assert!(body.contains("name"));
        assert!(body.contains("notes"));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct NestedArgs {
        label: String,
        options: NestedOptions,
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct NestedOptions {
        timeout: u32,
        verbose: bool,
    }

    #[test]
    fn nested_object_compiles() {
        let result = compile_for::<NestedArgs>("test:nested");
        assert!(result.is_some());
        let body = get_main_rule(&result);
        assert!(body.contains("label"));
        assert!(body.contains("options"));
    }
}
