//! Lower a `schemars` Draft-7 [`RootSchema`] into the JSON-Schema subset accepted by OpenAI
//! strict structured outputs. Analogous to [`crate::engine::grammar::schema_to_gbnf_rule`].
//!
//! A typed, closed DTO ([`OpenAiSchema`]) models *only* the accepted subset and renders exactly
//! the right JSON, so the invariants (`additionalProperties: false`, all properties `required`,
//! optionals expressed as nullable unions) are unrepresentable-to-violate rather than enforced by
//! hand. Unsupported constructs return `Err`; the per-tool caller falls back to a permissive
//! empty-object schema (same graceful-degradation philosophy as `schema_to_gbnf`).

use schemars::schema::{
    ArrayValidation, InstanceType, ObjectValidation, RootSchema, Schema, SchemaObject, SingleOrVec,
    SubschemaValidation,
};
use serde_json::{Value, json};

use crate::executive::error::{FcpError, Result};

/// Closed model of the OpenAI-strict JSON-Schema subset.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenAiSchema {
    /// Always renders `additionalProperties: false` with **every** property listed in `required`
    /// (strict-mode invariant). Optional Rust fields are lowered as [`Self::Nullable`].
    Object {
        /// Sorted by key for deterministic output (mirrors the GBNF compiler).
        properties: Vec<(String, OpenAiSchema)>,
    },
    String {
        enum_values: Option<Vec<String>>,
    },
    Integer,
    Number,
    Boolean,
    Null,
    Array {
        items: Box<OpenAiSchema>,
    },
    /// Renders as `anyOf: [inner, {"type": "null"}]` — how strict mode expresses optionality.
    Nullable(Box<OpenAiSchema>),
}

impl OpenAiSchema {
    /// Permissive fallback for tools whose schema cannot be lowered: an empty object
    /// (strict mode forbids free-form objects, so "no constrained args" is the safe shape).
    #[must_use]
    pub fn empty_object() -> Self {
        Self::Object {
            properties: Vec::new(),
        }
    }

    /// Render to the exact JSON accepted by `response_format: json_schema (strict)`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Object { properties } => {
                let mut props = serde_json::Map::new();
                let mut required: Vec<Value> = Vec::with_capacity(properties.len());
                for (key, schema) in properties {
                    props.insert(key.clone(), schema.to_value());
                    required.push(Value::String(key.clone()));
                }
                json!({
                    "type": "object",
                    "properties": Value::Object(props),
                    "required": required,
                    "additionalProperties": false,
                })
            }
            Self::String { enum_values } => match enum_values {
                Some(values) => json!({ "type": "string", "enum": values }),
                None => json!({ "type": "string" }),
            },
            Self::Integer => json!({ "type": "integer" }),
            Self::Number => json!({ "type": "number" }),
            Self::Boolean => json!({ "type": "boolean" }),
            Self::Null => json!({ "type": "null" }),
            Self::Array { items } => json!({ "type": "array", "items": items.to_value() }),
            Self::Nullable(inner) => json!({ "anyOf": [inner.to_value(), { "type": "null" }] }),
        }
    }
}

const MAX_DEPTH: u8 = 8;

struct LowerCtx<'a> {
    definitions: &'a schemars::Map<String, Schema>,
    depth: u8,
}

fn unsupported(what: &str) -> FcpError {
    FcpError::SchemaViolation(format!("schema_to_openai: unsupported construct: {what}"))
}

/// Lower a tool's full [`RootSchema`] (inlining `#/definitions/…` refs).
pub fn lower_root_schema(schema: &RootSchema) -> Result<OpenAiSchema> {
    let mut ctx = LowerCtx {
        definitions: &schema.definitions,
        depth: 0,
    };
    lower_schema_object(&schema.schema, &mut ctx)
}

/// Per-tool entry point with graceful degradation: falls back to [`OpenAiSchema::empty_object`]
/// (logging the reason) when the schema cannot be lowered. Because tool args are typed Rust
/// structs deriving `JsonSchema`, most tools lower cleanly.
#[must_use]
pub fn tool_args_schema(tool_name: &str, schema: &RootSchema) -> OpenAiSchema {
    match lower_root_schema(schema) {
        Ok(lowered) => lowered,
        Err(e) => {
            tracing::warn!(
                tool = tool_name,
                error = %e,
                "schema_to_openai: falling back to permissive empty-object args schema"
            );
            OpenAiSchema::empty_object()
        }
    }
}

fn lower_schema_object(schema: &SchemaObject, ctx: &mut LowerCtx<'_>) -> Result<OpenAiSchema> {
    if let Some(subs) = schema.subschemas.as_ref()
        && (subs.one_of.is_some() || subs.any_of.is_some() || subs.all_of.is_some())
    {
        return lower_subschemas(subs, ctx);
    }

    if let Some(ref reference) = schema.reference {
        return resolve_ref(reference, ctx);
    }

    let instance_type = match &schema.instance_type {
        Some(SingleOrVec::Single(t)) => Some(**t),
        Some(SingleOrVec::Vec(types)) => {
            if types.len() == 2 && types.contains(&InstanceType::Null) {
                let non_null = types
                    .iter()
                    .find(|t| **t != InstanceType::Null)
                    .ok_or_else(|| unsupported("type: [null, null]"))?;
                let mut inner = schema.clone();
                inner.instance_type = Some(SingleOrVec::Single(Box::new(*non_null)));
                return Ok(OpenAiSchema::Nullable(Box::new(lower_schema_object(
                    &inner, ctx,
                )?)));
            }
            return Err(unsupported("multi-type (non-nullable) union"));
        }
        None => None,
    };

    match instance_type {
        Some(InstanceType::Object) => lower_object(schema.object.as_deref(), ctx),
        Some(InstanceType::String) => Ok(lower_string(schema)),
        Some(InstanceType::Integer) => Ok(OpenAiSchema::Integer),
        Some(InstanceType::Number) => Ok(OpenAiSchema::Number),
        Some(InstanceType::Boolean) => Ok(OpenAiSchema::Boolean),
        Some(InstanceType::Null) => Ok(OpenAiSchema::Null),
        Some(InstanceType::Array) => lower_array(schema.array.as_deref(), ctx),
        None => {
            if schema.enum_values.is_some() {
                Ok(lower_string(schema))
            } else if schema.object.is_some() {
                lower_object(schema.object.as_deref(), ctx)
            } else {
                Err(unsupported(
                    "schema with no instance_type and no recognizable shape",
                ))
            }
        }
    }
}

fn resolve_ref(reference: &str, ctx: &mut LowerCtx<'_>) -> Result<OpenAiSchema> {
    let def_name = reference
        .strip_prefix("#/definitions/")
        .ok_or_else(|| unsupported("non-local $ref"))?;
    let definition = ctx
        .definitions
        .get(def_name)
        .ok_or_else(|| unsupported("unresolved $ref"))?;
    if ctx.depth >= MAX_DEPTH {
        return Err(unsupported("nesting/$ref depth exceeded"));
    }
    ctx.depth += 1;
    let out = match definition {
        Schema::Object(obj) => lower_schema_object(obj, ctx),
        Schema::Bool(_) => Err(unsupported("boolean schema definition")),
    };
    ctx.depth -= 1;
    out
}

/// Lower the `oneOf`/`anyOf`/`allOf` shapes schemars emits for annotated fields and
/// `Option<T>` over refs/enums. Handles exactly the two lossless wrappers:
/// - single-element `allOf` (schemars' pattern for an annotated `$ref` / field type)
///   -> unwrap and lower the wrapped schema;
/// - two-arm nullable `anyOf`/`oneOf` (`[T, null]` in any order, e.g. `Option<Enum>`)
///   -> [`OpenAiSchema::Nullable`] over the non-null arm.
///
/// Anything else (multi-element `allOf`, a genuine >2 / non-nullable union) is a real
/// unsupported construct and returns `Err`, so [`tool_args_schema`] logs and falls back.
fn lower_subschemas(subs: &SubschemaValidation, ctx: &mut LowerCtx<'_>) -> Result<OpenAiSchema> {
    // `allOf: [ T ]` — schemars wraps a single annotated `$ref`/schema this way. A single
    // element is a pure wrapper we can unwrap losslessly; more than one is a real intersection
    // we cannot represent in the strict subset.
    if let Some(all_of) = subs.all_of.as_deref() {
        return match all_of {
            [single] => lower_schema(single, ctx),
            _ => Err(unsupported("multi-element allOf")),
        };
    }

    // `anyOf`/`oneOf` — schemars renders `Option<T>` over a ref/enum as `[T, {"type":"null"}]`.
    // Mirror the `[T, null]` instance-type branch already handled for scalars.
    let arms = subs
        .any_of
        .as_deref()
        .or(subs.one_of.as_deref())
        .ok_or_else(|| unsupported("empty subschema"))?;
    match nullable_inner(arms) {
        Some(inner) => Ok(OpenAiSchema::Nullable(Box::new(lower_schema(inner, ctx)?))),
        None => Err(unsupported("non-nullable oneOf/anyOf union")),
    }
}

/// If `arms` is a two-arm nullable union (`[T, null]` in any order), return the non-null arm.
fn nullable_inner(arms: &[Schema]) -> Option<&Schema> {
    if arms.len() != 2 || !arms.iter().any(is_null_schema) {
        return None;
    }
    arms.iter().find(|s| !is_null_schema(s))
}

/// True for a schema node that is exactly `{ "type": "null" }`.
fn is_null_schema(schema: &Schema) -> bool {
    matches!(
        schema,
        Schema::Object(obj)
            if matches!(
                obj.instance_type,
                Some(SingleOrVec::Single(ref t)) if **t == InstanceType::Null
            )
    )
}

/// Lower a `Schema` node (bool or object) via [`lower_schema_object`].
fn lower_schema(schema: &Schema, ctx: &mut LowerCtx<'_>) -> Result<OpenAiSchema> {
    match schema {
        Schema::Object(obj) => lower_schema_object(obj, ctx),
        Schema::Bool(_) => Err(unsupported("boolean schema in subschema arm")),
    }
}

fn lower_string(schema: &SchemaObject) -> OpenAiSchema {
    let enum_values = schema.enum_values.as_ref().and_then(|values| {
        let strings: Vec<String> = values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        (!strings.is_empty()).then_some(strings)
    });
    OpenAiSchema::String { enum_values }
}

fn lower_object(
    validation: Option<&ObjectValidation>,
    ctx: &mut LowerCtx<'_>,
) -> Result<OpenAiSchema> {
    let Some(validation) = validation else {
        return Ok(OpenAiSchema::empty_object());
    };

    let free_form_additional = validation
        .additional_properties
        .as_deref()
        .is_some_and(|s| !matches!(s, Schema::Bool(false)));
    if free_form_additional && validation.properties.is_empty() {
        return Err(unsupported("free-form additionalProperties"));
    }

    if ctx.depth >= MAX_DEPTH {
        return Err(unsupported("nesting depth exceeded"));
    }
    ctx.depth += 1;

    let required: std::collections::HashSet<&str> =
        validation.required.iter().map(String::as_str).collect();

    let mut keys: Vec<&String> = validation.properties.keys().collect();
    keys.sort();

    let mut properties: Vec<(String, OpenAiSchema)> = Vec::with_capacity(keys.len());
    let mut result: Result<()> = Ok(());
    for key in keys {
        let prop = match validation.properties.get(key) {
            Some(Schema::Object(obj)) => lower_schema_object(obj, ctx),
            Some(Schema::Bool(true)) => Err(unsupported("any-typed property")),
            Some(Schema::Bool(false)) | None => Err(unsupported("false/missing property schema")),
        };
        match prop {
            Ok(lowered) => {
                // Strict mode lists every property in `required`; a schemars-optional field
                // becomes nullable so the model can express "absent" as null.
                let lowered = if required.contains(key.as_str())
                    || matches!(lowered, OpenAiSchema::Nullable(_))
                {
                    lowered
                } else {
                    OpenAiSchema::Nullable(Box::new(lowered))
                };
                properties.push((key.clone(), lowered));
            }
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }

    ctx.depth -= 1;
    result?;
    Ok(OpenAiSchema::Object { properties })
}

fn lower_array(
    validation: Option<&ArrayValidation>,
    ctx: &mut LowerCtx<'_>,
) -> Result<OpenAiSchema> {
    let items = match validation.and_then(|v| v.items.as_ref()) {
        Some(SingleOrVec::Single(item_schema)) => match item_schema.as_ref() {
            Schema::Object(obj) => lower_schema_object(obj, ctx)?,
            Schema::Bool(_) => return Err(unsupported("boolean array item schema")),
        },
        Some(SingleOrVec::Vec(_)) => return Err(unsupported("tuple-typed array items")),
        None => return Err(unsupported("array without item schema")),
    };
    Ok(OpenAiSchema::Array {
        items: Box::new(items),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::Deserialize;

    fn lower_for<T: JsonSchema>() -> Result<OpenAiSchema> {
        lower_root_schema(&schemars::schema_for!(T))
    }

    fn value_for<T: JsonSchema>() -> Value {
        lower_for::<T>().expect("lower").to_value()
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct SimpleRequired {
        relative_path: String,
    }

    #[test]
    fn simple_required_string_field() {
        let v = value_for::<SimpleRequired>();
        assert_eq!(v["type"], "object");
        assert_eq!(v["additionalProperties"], false);
        assert_eq!(v["properties"]["relative_path"]["type"], "string");
        assert_eq!(v["required"], json!(["relative_path"]));
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct WithOptional {
        required_field: String,
        optional_field: Option<String>,
    }

    #[test]
    fn optional_field_becomes_nullable_but_stays_required() {
        let v = value_for::<WithOptional>();
        let required: Vec<&str> = v["required"]
            .as_array()
            .expect("required array")
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert!(required.contains(&"required_field"));
        assert!(
            required.contains(&"optional_field"),
            "strict mode lists every property in required"
        );
        let opt = &v["properties"]["optional_field"];
        let any_of = opt["anyOf"].as_array().expect("anyOf union for optional");
        assert!(any_of.iter().any(|s| s["type"] == "null"));
        assert!(any_of.iter().any(|s| s["type"] == "string"));
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
    fn enum_field_inlines_definitions_ref() {
        let v = value_for::<WithEnum>();
        let mode = &v["properties"]["mode"];
        assert_eq!(mode["type"], "string");
        assert_eq!(mode["enum"], json!(["overwrite", "append"]));
        assert!(!v.to_string().contains("$ref"), "refs must be inlined: {v}");
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct MixedScalars {
        minutes: u32,
        ratio: f64,
        permanent: bool,
        tags: Vec<String>,
    }

    #[test]
    fn scalar_and_array_types() {
        let v = value_for::<MixedScalars>();
        assert_eq!(v["properties"]["minutes"]["type"], "integer");
        assert_eq!(v["properties"]["ratio"]["type"], "number");
        assert_eq!(v["properties"]["permanent"]["type"], "boolean");
        assert_eq!(v["properties"]["tags"]["type"], "array");
        assert_eq!(v["properties"]["tags"]["items"]["type"], "string");
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct NestedOptions {
        timeout: u32,
        verbose: bool,
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct NestedArgs {
        label: String,
        options: NestedOptions,
    }

    #[test]
    fn nested_object_all_levels_closed() {
        let v = value_for::<NestedArgs>();
        let options = &v["properties"]["options"];
        assert_eq!(options["type"], "object");
        assert_eq!(options["additionalProperties"], false);
        assert_eq!(options["properties"]["timeout"]["type"], "integer");
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct EmptyArgs {}

    #[test]
    fn empty_args_is_closed_empty_object() {
        let v = value_for::<EmptyArgs>();
        assert_eq!(v["type"], "object");
        assert_eq!(v["additionalProperties"], false);
        assert_eq!(v["required"], json!([]));
    }

    #[test]
    fn unsupported_oneof_errors_and_tool_fallback_is_empty_object() {
        use schemars::schema::SubschemaValidation;
        let mut root = schemars::schema_for!(EmptyArgs);
        root.schema.subschemas = Some(Box::new(SubschemaValidation {
            one_of: Some(vec![Schema::Bool(true)]),
            ..Default::default()
        }));
        assert!(lower_root_schema(&root).is_err());
        let fallback = tool_args_schema("test:unsupported", &root);
        assert_eq!(fallback, OpenAiSchema::empty_object());
    }

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    struct FreeFormArgs {
        payload: serde_json::Value,
    }

    #[test]
    fn free_form_value_field_is_unsupported() {
        assert!(lower_for::<FreeFormArgs>().is_err());
    }

    #[test]
    fn properties_sorted_deterministically() {
        #[derive(JsonSchema, Deserialize)]
        #[allow(dead_code)]
        struct Unsorted {
            zebra: String,
            alpha: String,
        }
        let v = value_for::<Unsorted>();
        let keys: Vec<&String> = v["properties"]
            .as_object()
            .expect("object")
            .keys()
            .collect();
        assert_eq!(keys, vec!["alpha", "zebra"]);
    }

    // --- Phase 0: subschema (allOf/anyOf/oneOf) lowering ---------------------
    //
    // schemars renders `Option<T>`, fieldless enums, and annotated `$ref` field types via
    // `oneOf`/`anyOf`/`allOf`. Before this fix any such construct dropped the whole tool to a
    // permissive empty-object schema, so `memory:query` et al. lost their real args and failed
    // Gatekeeper's required-field validation (vault `billy`).

    #[derive(JsonSchema, Deserialize)]
    #[allow(dead_code)]
    #[serde(rename_all = "snake_case")]
    enum SortArg {
        Semantic,
        Recency,
    }

    #[test]
    fn option_string_field_lowers_to_nullable() {
        // `Option<String>` renders as `{"type": ["string", "null"]}` — the scalar `[T, null]`
        // instance-type branch. It must lower to a nullable string, never fall back.
        #[derive(JsonSchema, Deserialize)]
        #[allow(dead_code)]
        struct Args {
            note: Option<String>,
        }
        let lowered = lower_for::<Args>().expect("Option<String> must lower");
        let OpenAiSchema::Object { properties } = &lowered else {
            panic!("expected object, got {lowered:?}");
        };
        let (_, note) = properties
            .iter()
            .find(|(k, _)| k == "note")
            .expect("note property");
        assert!(
            matches!(note, OpenAiSchema::Nullable(inner) if matches!(**inner, OpenAiSchema::String { .. })),
            "note must be Nullable(String), got {note:?}"
        );
    }

    #[test]
    fn allof_ref_wrapper_unwraps() {
        // A required enum field carrying a doc comment renders as `allOf: [ { $ref } ]`.
        // The single-element wrapper must unwrap to the referenced string enum.
        #[derive(JsonSchema, Deserialize)]
        #[allow(dead_code)]
        struct Args {
            /// doc comment forces schemars to wrap the `$ref` in `allOf` to attach it.
            sort: SortArg,
        }
        let lowered = lower_for::<Args>().expect("allOf wrapper must lower");
        let OpenAiSchema::Object { properties } = &lowered else {
            panic!("expected object, got {lowered:?}");
        };
        let (_, sort) = properties
            .iter()
            .find(|(k, _)| k == "sort")
            .expect("sort property");
        // `sort` is required-with-default => optional in JSON Schema => wrapped Nullable here.
        let inner = match sort {
            OpenAiSchema::Nullable(inner) => inner.as_ref(),
            other => other,
        };
        match inner {
            OpenAiSchema::String {
                enum_values: Some(values),
            } => assert_eq!(values, &vec!["semantic".to_string(), "recency".to_string()]),
            other => panic!("expected string enum, got {other:?}"),
        }
        assert!(
            !lowered.to_value().to_string().contains("$ref"),
            "refs must be inlined"
        );
    }

    #[test]
    fn fieldless_enum_ref_lowers_to_string_enum() {
        // A required enum field WITHOUT annotations renders as a bare `$ref` (no allOf wrapper).
        #[derive(JsonSchema, Deserialize)]
        #[allow(dead_code)]
        struct Args {
            sort: SortArg,
        }
        let v = value_for::<Args>();
        assert_eq!(v["properties"]["sort"]["type"], "string");
        assert_eq!(
            v["properties"]["sort"]["enum"],
            json!(["semantic", "recency"])
        );
        assert!(!v.to_string().contains("$ref"), "refs must be inlined: {v}");
    }

    #[test]
    fn anyof_ref_null_lowers_to_nullable_enum() {
        // `Option<Enum>` with a doc comment renders as `anyOf: [ { $ref }, { "type": "null" } ]`.
        #[derive(JsonSchema, Deserialize)]
        #[allow(dead_code)]
        struct Args {
            /// optional enum with annotation => anyOf[ref, null]
            sort: Option<SortArg>,
        }
        let lowered = lower_for::<Args>().expect("anyOf[ref, null] must lower");
        let OpenAiSchema::Object { properties } = &lowered else {
            panic!("expected object, got {lowered:?}");
        };
        let (_, sort) = properties
            .iter()
            .find(|(k, _)| k == "sort")
            .expect("sort property");
        let OpenAiSchema::Nullable(inner) = sort else {
            panic!("expected Nullable, got {sort:?}");
        };
        assert!(
            matches!(
                **inner,
                OpenAiSchema::String {
                    enum_values: Some(_)
                }
            ),
            "inner must be a string enum, got {inner:?}"
        );
        // Must NOT double-wrap into Nullable(Nullable(_)).
        assert!(
            !matches!(**inner, OpenAiSchema::Nullable(_)),
            "nullable enum must not be double-wrapped"
        );
    }

    #[test]
    fn memory_query_lowers_with_required_query() {
        // The real regression from vault `billy`: `memory:query` must lower to a real object
        // with a required `query` string — not the permissive empty-object fallback.
        let root = schemars::schema_for!(crate::tools::memory::query::MemoryQueryArgs);
        let lowered = lower_root_schema(&root).expect("MemoryQueryArgs must lower cleanly");
        assert_ne!(
            lowered,
            OpenAiSchema::empty_object(),
            "must not collapse to permissive empty-object"
        );
        let v = lowered.to_value();
        assert_eq!(v["type"], "object");
        assert_eq!(v["additionalProperties"], false);
        assert_eq!(v["properties"]["query"]["type"], "string");
        let required: Vec<&str> = v["required"]
            .as_array()
            .expect("required array")
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert!(required.contains(&"query"), "query must be required: {v}");
        // The enum arg that previously triggered the fallback is present and inlined.
        assert!(v["properties"].get("memory_sort").is_some());
        assert!(!v.to_string().contains("$ref"), "refs must be inlined");
    }

    #[test]
    fn memory_stage_lowers_with_nullable_enum_args() {
        // `memory:stage` carries `Option<Enum>` (`kind`/`tier`) => anyOf[ref, null]; must lower.
        let root = schemars::schema_for!(crate::tools::memory::stage::MemoryStageArgs);
        let lowered = lower_root_schema(&root).expect("MemoryStageArgs must lower cleanly");
        assert_ne!(lowered, OpenAiSchema::empty_object());
        let v = lowered.to_value();
        let kind = &v["properties"]["kind"];
        let any_of = kind["anyOf"].as_array().expect("kind anyOf");
        assert!(any_of.iter().any(|s| s["type"] == "null"));
        assert!(any_of.iter().any(|s| s["enum"].is_array()));
        assert!(!v.to_string().contains("$ref"), "refs must be inlined");
    }

    #[test]
    fn memory_staged_list_option_bool_lowers_without_fallback() {
        // `memory:staged_list` is one of the doc-named tools; its `Option<bool>` field
        // (`include_content_preview`) previously contributed to the empty-object collapse. It
        // must now lower to a real object with a nullable boolean. (Other doc-named tools such as
        // `news:today` / `db:find_connections` live in private modules and are exercised via the
        // synthetic `allOf`/`anyOf`-nullable tests above; `doc:list` is intentionally free-form
        // `serde_json::Value`, for which empty-object is the correct lowering.)
        let staged = schemars::schema_for!(crate::tools::memory::staged_list::MemoryStagedListArgs);
        let lowered = lower_root_schema(&staged).expect("memory:staged_list lowers");
        assert_ne!(lowered, OpenAiSchema::empty_object());
        let preview = lowered.to_value()["properties"]["include_content_preview"].clone();
        let any_of = preview["anyOf"].as_array().expect("nullable anyOf");
        assert!(any_of.iter().any(|s| s["type"] == "null"));
        assert!(any_of.iter().any(|s| s["type"] == "boolean"));
    }

    #[test]
    fn genuinely_unsupported_multi_arm_union_still_falls_back() {
        // A real >2-arm / non-nullable union is unrepresentable in the strict subset and must
        // still degrade gracefully to the permissive empty-object (with a warn, tested via the
        // per-tool entry point).
        use schemars::schema::SubschemaValidation;
        let mut root = schemars::schema_for!(EmptyArgs);
        root.schema.subschemas = Some(Box::new(SubschemaValidation {
            any_of: Some(vec![
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    ..Default::default()
                }),
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Integer))),
                    ..Default::default()
                }),
            ]),
            ..Default::default()
        }));
        assert!(lower_root_schema(&root).is_err());
        assert_eq!(
            tool_args_schema("test:multi_union", &root),
            OpenAiSchema::empty_object()
        );
    }
}
