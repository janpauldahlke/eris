//! Shared Gmail list-row formatting (metadata) for `mail:check` and `mail:digest`.

use serde_json::Value;

pub fn header_value_from_json(headers: &[Value], name: &str) -> Option<String> {
    for h in headers {
        let n = h.get("name").and_then(|x| x.as_str())?;
        if n.eq_ignore_ascii_case(name) {
            return h.get("value").and_then(|x| x.as_str()).map(String::from);
        }
    }
    None
}

pub fn format_metadata_line(v: &Value, fallback_id: &str, fallback_thread: &str) -> String {
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or(fallback_id);
    let thread = v
        .get("threadId")
        .and_then(|x| x.as_str())
        .unwrap_or(fallback_thread);
    let snippet: String = v
        .get("snippet")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect();
    let preview = if snippet.is_empty() {
        "(no preview)".to_string()
    } else {
        snippet
    };

    let (subject, from, date) = v
        .get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(|h| h.as_array())
        .map(|headers| {
            (
                header_value_from_json(headers, "Subject").unwrap_or_else(|| "(no subject)".into()),
                header_value_from_json(headers, "From")
                    .unwrap_or_else(|| "(unknown sender)".into()),
                header_value_from_json(headers, "Date").unwrap_or_else(|| "".into()),
            )
        })
        .unwrap_or_else(|| {
            (
                "(no subject)".into(),
                "(unknown sender)".into(),
                String::new(),
            )
        });

    format!(
        "- id={id} | thread={thread} | subject: {subject} | from: {from} | date: {date} | preview: {preview}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_metadata_line_includes_subject_and_from() {
        let v = json!({
            "id": "msg1",
            "threadId": "th1",
            "snippet": "Hello there",
            "payload": {
                "headers": [
                    {"name": "Subject", "value": "Meeting tomorrow"},
                    {"name": "From", "value": "alice@example.com"},
                    {"name": "Date", "value": "Mon, 6 Apr 2026 12:00:00 +0000"}
                ]
            }
        });
        let line = format_metadata_line(&v, "msg1", "th1");
        assert!(line.contains("subject: Meeting tomorrow"));
        assert!(line.contains("from: alice@example.com"));
        assert!(line.contains("preview: Hello there"));
        assert!(line.contains("thread=th1"));
    }
}
