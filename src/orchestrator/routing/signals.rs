//! Lightweight signal extraction for pre-LLM routing (Phase 2).
//!
//! No new ML — boolean / lexical cues plus embed hits and dialog context.

/// Snapshot of inputs the policy rules may read.
#[derive(Debug, Clone)]
pub struct RoutingSignals {
    pub user_text: String,
    /// Router hits already sorted descending (embed + lexical forced).
    pub embed_hits: Vec<(String, f32)>,
    /// Session-scoped recent successes (newest last).
    pub recent_successful_tools: Vec<String>,
    pub has_url: bool,
    pub agenda_continuation: bool,
    pub doc_ingest_cues: bool,
    pub recent_had_agenda: bool,
    pub recent_had_mail: bool,
    pub recent_had_calendar: bool,
    /// Recent `doc:list` / `doc:read` / `doc:query` (catalog-ish), not bare ingest.
    pub recent_had_doc_catalog: bool,
    pub mail_delete_continuation: bool,
    pub mail_move_continuation: bool,
    pub mail_reply_continuation: bool,
    pub mail_read_continuation: bool,
    pub calendar_delete_continuation: bool,
    pub calendar_update_continuation: bool,
    pub calendar_get_continuation: bool,
    /// Document-anchored delete/remove (not bare "remove it").
    pub doc_delete_continuation: bool,
}

impl RoutingSignals {
    #[must_use]
    pub fn from_turn(
        user_text: &str,
        embed_hits: Vec<(String, f32)>,
        recent_successful_tools: &[String],
    ) -> Self {
        let lower = user_text.to_ascii_lowercase();
        let has_url =
            lower.contains("http://") || lower.contains("https://") || lower.contains("www.");
        let recent_had_agenda = recent_successful_tools
            .iter()
            .any(|n| n.starts_with("agenda:"));
        let recent_had_mail = recent_successful_tools
            .iter()
            .any(|n| n.starts_with("mail:"));
        let recent_had_calendar = recent_successful_tools
            .iter()
            .any(|n| n.starts_with("calendar:"));
        let recent_had_doc_catalog = recent_successful_tools.iter().any(|n| {
            matches!(
                n.as_str(),
                "doc:list" | "doc:read" | "doc:query" | "doc:delete"
            )
        });
        Self {
            user_text: user_text.to_string(),
            embed_hits,
            recent_successful_tools: recent_successful_tools.to_vec(),
            has_url,
            agenda_continuation: has_agenda_continuation_intent(user_text),
            doc_ingest_cues: has_doc_ingest_cues(user_text),
            recent_had_agenda,
            recent_had_mail,
            recent_had_calendar,
            recent_had_doc_catalog,
            mail_delete_continuation: has_mail_delete_continuation(&lower),
            mail_move_continuation: has_mail_move_continuation(&lower),
            mail_reply_continuation: has_mail_reply_continuation(&lower),
            mail_read_continuation: has_mail_read_continuation(&lower),
            calendar_delete_continuation: has_calendar_delete_continuation(&lower),
            calendar_update_continuation: has_calendar_update_continuation(&lower),
            calendar_get_continuation: has_calendar_get_continuation(&lower),
            doc_delete_continuation: has_doc_delete_continuation(&lower),
        }
    }
}

/// User wants to close/remove/finish something after seeing agenda.
#[must_use]
pub fn has_agenda_continuation_intent(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const CUES: &[&str] = &[
        "remove",
        "delete",
        "done",
        "complete",
        "finished",
        "finish",
        "clear it",
        "clear that",
        "mark as done",
        "check off",
        "crossed off",
        "take it off",
        "off the agenda",
        "from the agenda",
        "from my agenda",
    ];
    CUES.iter().any(|c| lower.contains(c))
}

/// Explicit document-ingest intent — do not steal the turn for agenda pairing.
#[must_use]
pub fn has_doc_ingest_cues(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("ingest")
        || lower.contains("upload")
        || lower.contains(".pdf")
        || lower.contains("99_user_uploaded")
        || lower.contains("document rag")
        || (lower.contains("document")
            && (lower.contains("add") || lower.contains("index") || lower.contains("import")))
}

fn has_mail_noun(lower: &str) -> bool {
    lower.contains("email")
        || lower.contains("e-mail")
        || lower.contains("mail")
        || lower.contains("inbox")
        || lower.contains("message")
}

fn has_mail_delete_continuation(lower: &str) -> bool {
    let destructive = lower.contains("delete")
        || lower.contains("trash")
        || lower.contains("discard")
        || lower.contains("get rid of");
    destructive && has_mail_noun(lower)
}

fn has_mail_move_continuation(lower: &str) -> bool {
    let move_cue = lower.contains("move")
        || lower.contains("archive")
        || lower.contains("label")
        || lower.contains("file under")
        || lower.contains("spam");
    move_cue && has_mail_noun(lower)
}

fn has_mail_reply_continuation(lower: &str) -> bool {
    if lower.contains("moltbook") {
        return false;
    }
    if lower.contains("reply to that email")
        || lower.contains("reply via gmail")
        || lower.contains("reply to the email")
        || lower.contains("write back")
    {
        return true;
    }
    (lower.contains("reply") || lower.contains("respond")) && has_mail_noun(lower)
}

fn has_mail_read_continuation(lower: &str) -> bool {
    let read_cue = lower.contains("read")
        || lower.contains("open")
        || lower.contains("show full")
        || lower.contains("full message")
        || lower.contains("that message");
    read_cue && has_mail_noun(lower)
}

fn has_calendar_noun(lower: &str) -> bool {
    lower.contains("meeting")
        || lower.contains("event")
        || lower.contains("appointment")
        || lower.contains("calendar")
        || lower.contains("invite")
}

fn has_calendar_delete_continuation(lower: &str) -> bool {
    let destructive =
        lower.contains("cancel") || lower.contains("delete") || lower.contains("remove");
    destructive && has_calendar_noun(lower)
}

fn has_calendar_update_continuation(lower: &str) -> bool {
    let update = lower.contains("reschedule")
        || lower.contains("change time")
        || lower.contains("move the meeting")
        || lower.contains("update")
        || lower.contains("rename");
    update && has_calendar_noun(lower)
}

fn has_calendar_get_continuation(lower: &str) -> bool {
    let detail = lower.contains("details")
        || lower.contains("attendees")
        || lower.contains("meet link")
        || lower.contains("open that")
        || lower.contains("show that event");
    detail && (has_calendar_noun(lower) || lower.contains("that"))
}

/// Document-anchored delete — requires document/pdf/ingest vocabulary (not bare remove).
#[must_use]
pub fn has_doc_delete_continuation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let destructive =
        lower.contains("delete") || lower.contains("remove") || lower.contains("unindex");
    let doc_noun = lower.contains("document")
        || lower.contains("pdf")
        || lower.contains("ingested")
        || lower.contains("upload")
        || lower.contains("from rag")
        || lower.contains("from the store");
    destructive && doc_noun
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agenda_cues_and_doc_cues() {
        assert!(has_agenda_continuation_intent("done"));
        assert!(has_agenda_continuation_intent("remove that please"));
        assert!(!has_agenda_continuation_intent("what's the weather"));
        assert!(has_doc_ingest_cues("ingest report.pdf please"));
        assert!(!has_doc_ingest_cues("remove it"));
    }

    #[test]
    fn mail_and_doc_continuation_gates() {
        assert!(has_mail_delete_continuation(
            "please delete that email from my inbox"
        ));
        assert!(!has_mail_delete_continuation("please delete that"));
        assert!(has_doc_delete_continuation(
            "please delete that document from the store"
        ));
        assert!(!has_doc_delete_continuation("please remove it"));
        assert!(has_calendar_delete_continuation(
            "cancel that meeting on my calendar"
        ));
        assert!(!has_calendar_delete_continuation("cancel that"));
    }
}
