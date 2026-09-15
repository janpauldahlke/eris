//! Phrase-style tool map for slim system prompts (README compendium shape, generated from code).

use crate::tools::descriptors::ToolDescriptorRegistry;
use crate::tools::routing_phrases::fallback_triggers;

/// Typical phrasing line for one tool (descriptor hints win, else fallback triggers, else description).
pub fn typical_phrasing_for_tool(
    name: &str,
    description: &str,
    descriptors: Option<&ToolDescriptorRegistry>,
) -> String {
    if let Some(registry) = descriptors
        && let Some(desc) = registry.get(name)
        && !desc.routing_hints.is_empty()
    {
        return desc.routing_hints.join(", ");
    }
    let fb = fallback_triggers(name);
    if fb.is_empty() {
        description.to_string()
    } else {
        fb.to_string()
    }
}

/// Truncate a phrase-map description. `max_chars == 0` keeps the full text.
pub fn shorten_tool_description(description: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return description.to_string();
    }
    if description.chars().count() <= max_chars {
        return description.to_string();
    }
    let take: String = description
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    format!("{take}…")
}

/// Markdown table: tool name, short description, typical phrasing (for LLM routing context).
pub fn build_phrase_compendium(
    descriptors: Option<&ToolDescriptorRegistry>,
    tool_rows: &[(String, String)],
    description_preview_chars: usize,
) -> String {
    if tool_rows.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = vec![
        "[FCP_TOOL_PHRASE_MAP]".to_string(),
        "Natural-language hooks per tool; use exact tool `name` in tool_calls.".to_string(),
        String::new(),
        "| Tool | Description (short) | Typical phrasing / triggers |".to_string(),
        "| ---- | ------------------- | --------------------------- |".to_string(),
    ];
    for (name, description) in tool_rows {
        let phrases = typical_phrasing_for_tool(name, description, descriptors);
        let desc_short = shorten_tool_description(description, description_preview_chars);
        let esc_name = name.replace('|', "\\|");
        let esc_desc = desc_short.replace('|', "\\|").replace('\n', " ");
        let esc_phr = phrases.replace('|', "\\|").replace('\n', " ");
        lines.push(format!("| **{esc_name}** | {esc_desc} | {esc_phr} |"));
    }
    lines.push(String::new());
    lines.push("[/FCP_TOOL_PHRASE_MAP]".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typical_phrasing_uses_fallback_for_clock_now() {
        let s = typical_phrasing_for_tool("clock:now", "Returns local time.", None);
        assert!(s.contains("time"));
    }

    #[test]
    fn phrase_compendium_contains_tool_name() {
        let rows = vec![("clock:now".to_string(), "Local time.".to_string())];
        let c = build_phrase_compendium(None, &rows, 120);
        assert!(c.contains("clock:now"));
        assert!(c.contains("[FCP_TOOL_PHRASE_MAP]"));
    }

    #[test]
    fn phrase_compendium_truncates_description_to_preview_chars() {
        let long = "UNIQUE_DESC_MARKER_".to_string() + &"x".repeat(200);
        let c = build_phrase_compendium(None, &[("clock:now".to_string(), long.clone())], 50);
        let preview = shorten_tool_description(&long, 50);
        assert_eq!(preview.chars().count(), 50);
        assert!(preview.ends_with('…'));
        assert!(
            c.contains(&format!("| **clock:now** | {preview} |")),
            "description column should be the truncated preview, got:\n{c}"
        );
        assert!(
            !c.contains(&long),
            "full description must not leak when fallback phrasing is used"
        );
    }

    #[test]
    fn phrase_compendium_zero_preview_chars_keeps_full_description() {
        let long = "x".repeat(200);
        let c = build_phrase_compendium(None, &[("vault:search".to_string(), long.clone())], 0);
        assert!(c.contains(&long));
    }
}
