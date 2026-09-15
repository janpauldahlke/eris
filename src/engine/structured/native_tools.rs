//! OpenAI-native `tools[]` function definitions for hosted backends.
//! Parameters come from the same [`super::tool_args_schema`] lowering as the envelope subset.

use serde::Serialize;
use serde_json::Value;

/// One OpenAI `tools[]` item (`type: "function"`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiNativeTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: OpenAiNativeFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiNativeFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}

impl OpenAiNativeTool {
    /// Strict function tool: `additionalProperties: false` + all-required is already
    /// guaranteed by [`super::OpenAiSchema::to_value`].
    #[must_use]
    pub fn function(name: String, description: String, parameters: Value) -> Self {
        Self {
            kind: "function".into(),
            function: OpenAiNativeFunction {
                name,
                description,
                parameters,
                strict: true,
            },
        }
    }
}
