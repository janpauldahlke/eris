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

    /// plan:* fallback triggers exist (workflow/sequence language, not todo-queue language).
    #[test]
    fn typical_phrasing_fallback_for_plan_tools_is_nonempty() {
        for (tool, expected) in [
            ("plan:read", "working plan"),
            ("plan:set", "multi-step"),
            ("plan:update", "advance the plan"),
        ] {
            let s = typical_phrasing_for_tool(tool, "desc", None);
            assert!(!s.is_empty(), "{tool} has empty fallback phrasing");
            assert!(s.contains(expected), "{tool} fallback missing {expected:?}: {s}");
        }
    }

    /// Descriptor `routing_hints` win over fallback triggers when the registry has them.
    #[test]
    fn typical_phrasing_prefers_descriptor_hints_for_plan_set() {
        let registry = crate::tools::descriptors::ToolDescriptorRegistry::load_embedded()
            .expect("embedded descriptors");
        let s = typical_phrasing_for_tool("plan:set", "desc", Some(&registry));
        assert!(
            s.contains("first then"),
            "descriptor routing_hints should win: {s}"
        );
    }

    /// agenda:push must not keep the bare "plan" todo token — that phrasing now belongs to plan:*.
    #[test]
    fn agenda_push_fallback_dropped_bare_plan_token() {
        let s = fallback_triggers("agenda:push");
        let tokens: Vec<&str> = s.split(',').map(str::trim).collect();
        assert!(!tokens.contains(&"plan"), "bare plan token still on agenda:push: {s}");
    }
}
