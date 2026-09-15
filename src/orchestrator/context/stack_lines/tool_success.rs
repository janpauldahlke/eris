//! Canonical encode/decode for tool success lines on [`crate::engine::Message::content`].

/// Wire prefix for a tool success system line (before the tool name).
pub const TOOL_SUCCESS_PREFIX: &str = "Tool '";
/// Wire infix between tool name and result body.
pub const TOOL_SUCCESS_INFIX: &str = "' succeeded: ";

/// Parsed tool success payload (borrowed view of the original `content` string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSuccessLine<'a> {
    pub tool_name: &'a str,
    pub body: &'a str,
}

/// Classifies a system `content` string for context transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedSystemLine<'a> {
    ToolSuccess(ToolSuccessLine<'a>),
    Other,
}

/// Format a tool success line for [`crate::orchestrator::core::tool_dispatch`].
pub fn format_tool_success_line(tool_name: &str, body: &str) -> String {
    let mut out = String::with_capacity(
        TOOL_SUCCESS_PREFIX.len() + tool_name.len() + TOOL_SUCCESS_INFIX.len() + body.len(),
    );
    out.push_str(TOOL_SUCCESS_PREFIX);
    out.push_str(tool_name);
    out.push_str(TOOL_SUCCESS_INFIX);
    out.push_str(body);
    out
}

/// Parse a tool success line; returns `None` if `content` does not match the protocol.
pub fn try_parse_tool_success_line(content: &str) -> Option<ToolSuccessLine<'_>> {
    let rest = content.strip_prefix(TOOL_SUCCESS_PREFIX)?;
    let (tool_name, body) = rest.split_once(TOOL_SUCCESS_INFIX)?;
    Some(ToolSuccessLine { tool_name, body })
}

/// True when this stack row is a tool success result: folded `system` line (local
/// backends) or native OpenRouter `role:"tool"` frame with the same content shape.
pub fn message_is_tool_success(m: &crate::engine::Message) -> Option<ToolSuccessLine<'_>> {
    if m.role == "system" || m.role == "tool" {
        try_parse_tool_success_line(&m.content)
    } else {
        None
    }
}

/// Parse a system line into [`ParsedSystemLine`].
pub fn parse_system_line(content: &str) -> ParsedSystemLine<'_> {
    match try_parse_tool_success_line(content) {
        Some(ts) => ParsedSystemLine::ToolSuccess(ts),
        None => ParsedSystemLine::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_round_trip() {
        let s = format_tool_success_line("t:1", "hello");
        let p = try_parse_tool_success_line(&s).expect("parse");
        assert_eq!(p.tool_name, "t:1");
        assert_eq!(p.body, "hello");
    }

    #[test]
    fn parse_colon_tool_name() {
        let s = format_tool_success_line("x:y", "payload");
        let p = try_parse_tool_success_line(&s).expect("parse");
        assert_eq!(p.tool_name, "x:y");
        assert_eq!(p.body, "payload");
    }

    #[test]
    fn parse_rejects_non_match() {
        assert!(try_parse_tool_success_line("Tool 'x' failed: z").is_none());
        assert!(try_parse_tool_success_line("not a tool line").is_none());
    }

    #[test]
    fn parsed_system_line_enum() {
        let s = format_tool_success_line("a", "b");
        match parse_system_line(&s) {
            ParsedSystemLine::ToolSuccess(t) => {
                assert_eq!(t.tool_name, "a");
                assert_eq!(t.body, "b");
            }
            ParsedSystemLine::Other => panic!("expected ToolSuccess"),
        }
        assert_eq!(parse_system_line("plain"), ParsedSystemLine::Other);
    }

    #[test]
    fn message_is_tool_success_accepts_system_and_tool_roles() {
        let line = format_tool_success_line("memory:query", "ok");
        let sys = crate::engine::Message::system(line.clone());
        let tool = crate::engine::Message::tool(line, "call_1");
        assert_eq!(
            message_is_tool_success(&sys).map(|t| t.tool_name),
            Some("memory:query")
        );
        assert_eq!(
            message_is_tool_success(&tool).map(|t| t.tool_name),
            Some("memory:query")
        );
        assert!(message_is_tool_success(&crate::engine::Message::user("hi")).is_none());
    }
}
