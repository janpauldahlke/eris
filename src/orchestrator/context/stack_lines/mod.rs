//! Typed helpers for well-known [`crate::engine::Message`] `content` patterns (wire format stays `String`).

mod tool_success;

pub use tool_success::{
    ParsedSystemLine, TOOL_SUCCESS_INFIX, TOOL_SUCCESS_PREFIX, ToolSuccessLine,
    format_tool_success_line, parse_system_line, try_parse_tool_success_line,
};
