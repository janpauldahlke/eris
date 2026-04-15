//! Formatting helpers for Google Calendar tool output.

use crate::generated::gws_types::calendar::{Event, EventDateTime};

pub(crate) fn format_event_datetime(dt: &Option<EventDateTime>) -> String {
    let Some(d) = dt else {
        return "?".to_string();
    };
    if let Some(ref t) = d.date_time {
        return t.clone();
    }
    if let Some(ref day) = d.date {
        return format!("{day} (all-day)");
    }
    "?".to_string()
}

pub(crate) fn format_event_one_line(ev: &Event) -> String {
    let id = ev.id.as_deref().unwrap_or("?");
    let summary = ev.summary.as_deref().unwrap_or("(no title)");
    let start = format_event_datetime(&ev.start);
    let end = format_event_datetime(&ev.end);
    format!("- id={id} | {summary} | {start} → {end}")
}
