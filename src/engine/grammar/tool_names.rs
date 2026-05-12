/// Build the GBNF alternation rule for valid tool names.
///
/// Each name becomes a quoted JSON string literal in GBNF.
/// Example: `["vault:read", "vault:write"]` produces
/// `"\"vault:read\"" | "\"vault:write\""`.
pub fn build_tool_name_enum(tool_names: &[String]) -> String {
    tool_names
        .iter()
        .map(|name| format!("\"\\\"{}\\\"\"", name))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_enum_formats_correctly() {
        let names = vec!["vault:read".into(), "vault:write".into()];
        let result = build_tool_name_enum(&names);
        assert_eq!(result, r#""\"vault:read\"" | "\"vault:write\"""#);
    }

    #[test]
    fn tool_name_enum_single_tool() {
        let names = vec!["web:fetch".into()];
        let result = build_tool_name_enum(&names);
        assert_eq!(result, r#""\"web:fetch\"""#);
    }

    #[test]
    fn tool_name_enum_empty() {
        let names: Vec<String> = vec![];
        let result = build_tool_name_enum(&names);
        assert!(result.is_empty());
    }
}
