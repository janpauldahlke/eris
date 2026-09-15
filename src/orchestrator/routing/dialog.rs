//! Dialog-continuation pairing: after a successful tool in a domain, map
//! follow-up phrasing onto that domain's cluster (and suppress weak cross-domain lock-in).

use super::clusters::expand_names_to_domain_clusters;
use super::decision::RoutingDecision;
use super::signals::RoutingSignals;

const FORCED_HIT_FLOOR: f32 = 0.99;

/// Ordered dialog-pairing rules. First match wins (agenda before mail/calendar/doc).
#[must_use]
pub fn try_dialog_pairing(
    signals: &RoutingSignals,
    registered: &[String],
) -> Option<RoutingDecision> {
    if let Some(d) = rule_agenda(signals, registered) {
        return Some(d);
    }
    if let Some(d) = rule_mail(signals, registered) {
        return Some(d);
    }
    if let Some(d) = rule_calendar(signals, registered) {
        return Some(d);
    }
    rule_doc(signals, registered)
}

fn rule_agenda(signals: &RoutingSignals, registered: &[String]) -> Option<RoutingDecision> {
    if !signals.recent_had_agenda || !signals.agenda_continuation || signals.doc_ingest_cues {
        return None;
    }
    // Prefer agenda over mail/calendar when recent agenda exists and cues match.
    let suppressed_doc = signals
        .embed_hits
        .iter()
        .any(|(n, s)| n.starts_with("doc:") && *s < FORCED_HIT_FLOOR);

    let offered = prefer_front(
        expand_names_to_domain_clusters(
            ["agenda:remove", "agenda:complete", "agenda:list"]
                .into_iter()
                .map(str::to_string),
            registered,
        ),
        &["agenda:remove", "agenda:complete", "agenda:list"],
        registered,
    );

    tracing::info!(
        suppressed_doc_hits = suppressed_doc,
        offered = ?offered,
        event = "routing.policy.dialog_pairing",
        domain = "agenda",
        "Agenda dialog continuation; offering agenda cluster"
    );

    Some(RoutingDecision::domain_cluster(
        "AGENDA_DIALOG_PAIRING",
        vec!["agenda"],
        offered,
    ))
}

fn rule_mail(signals: &RoutingSignals, registered: &[String]) -> Option<RoutingDecision> {
    if !signals.recent_had_mail || signals.doc_ingest_cues {
        return None;
    }
    // Destructive bare remove/delete without mail nouns → leave to agenda or ranked path.
    let preferred: &[&str] = if signals.mail_delete_continuation {
        &["mail:delete", "mail:check", "mail:digest"]
    } else if signals.mail_move_continuation {
        &["mail:move", "mail:check", "mail:read"]
    } else if signals.mail_reply_continuation {
        &["mail:write", "mail:read", "mail:check"]
    } else if signals.mail_read_continuation {
        &["mail:read", "mail:check", "mail:digest"]
    } else {
        return None;
    };

    let offered = prefer_front(
        expand_names_to_domain_clusters(preferred.iter().map(|s| (*s).to_string()), registered),
        preferred,
        registered,
    );

    tracing::info!(
        offered = ?offered,
        event = "routing.policy.dialog_pairing",
        domain = "mail",
        "Mail dialog continuation; offering mail cluster"
    );

    Some(RoutingDecision::domain_cluster(
        "MAIL_DIALOG_PAIRING",
        vec!["mail"],
        offered,
    ))
}

fn rule_calendar(signals: &RoutingSignals, registered: &[String]) -> Option<RoutingDecision> {
    if !signals.recent_had_calendar || signals.doc_ingest_cues {
        return None;
    }
    let preferred: &[&str] = if signals.calendar_delete_continuation {
        &["calendar:delete", "calendar:list", "calendar:get"]
    } else if signals.calendar_update_continuation {
        &["calendar:update", "calendar:get", "calendar:list"]
    } else if signals.calendar_get_continuation {
        &["calendar:get", "calendar:list"]
    } else {
        return None;
    };

    let offered = prefer_front(
        expand_names_to_domain_clusters(preferred.iter().map(|s| (*s).to_string()), registered),
        preferred,
        registered,
    );

    tracing::info!(
        offered = ?offered,
        event = "routing.policy.dialog_pairing",
        domain = "calendar",
        "Calendar dialog continuation; offering calendar cluster only (not full time affinity)"
    );

    Some(RoutingDecision::domain_cluster(
        "CALENDAR_DIALOG_PAIRING",
        vec!["calendar"],
        offered,
    ))
}

fn rule_doc(signals: &RoutingSignals, registered: &[String]) -> Option<RoutingDecision> {
    // Gated: require document-anchored delete language — bare "remove it" stays agenda/mail.
    if !signals.recent_had_doc_catalog || !signals.doc_delete_continuation {
        return None;
    }
    if signals.agenda_continuation && signals.recent_had_agenda {
        return None;
    }

    let preferred = ["doc:delete", "doc:list", "doc:query"];
    let offered = prefer_front(
        expand_names_to_domain_clusters(preferred.iter().map(|s| (*s).to_string()), registered),
        &preferred,
        registered,
    );

    tracing::info!(
        offered = ?offered,
        event = "routing.policy.dialog_pairing",
        domain = "doc",
        "Doc dialog continuation (document-anchored); offering doc cluster without ingest bias"
    );

    Some(RoutingDecision::domain_cluster(
        "DOC_DIALOG_PAIRING",
        vec!["doc"],
        offered,
    ))
}

fn prefer_front(
    mut offered: Vec<String>,
    preferred: &[&str],
    registered: &[String],
) -> Vec<String> {
    let mut front = Vec::new();
    for name in preferred {
        if let Some(pos) = offered.iter().position(|n| n == *name) {
            front.push(offered.remove(pos));
        } else if registered.iter().any(|n| n == *name) {
            front.push((*name).to_string());
        }
    }
    front.append(&mut offered);
    front
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::routing::decision::RoutingOffer;
    use crate::orchestrator::routing::signals::RoutingSignals;

    fn reg() -> Vec<String> {
        vec![
            "agenda:list".into(),
            "agenda:remove".into(),
            "agenda:complete".into(),
            "agenda:push".into(),
            "mail:check".into(),
            "mail:read".into(),
            "mail:delete".into(),
            "mail:move".into(),
            "mail:write".into(),
            "mail:digest".into(),
            "calendar:list".into(),
            "calendar:get".into(),
            "calendar:delete".into(),
            "calendar:update".into(),
            "doc:list".into(),
            "doc:delete".into(),
            "doc:query".into(),
            "doc:ingest".into(),
            "doc:read".into(),
        ]
    }

    fn signals(text: &str, recent: &[&str], hits: Vec<(String, f32)>) -> RoutingSignals {
        RoutingSignals::from_turn(
            text,
            hits,
            &recent.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn mail_delete_after_check() {
        let s = signals(
            "please delete that email from my inbox",
            &["mail:check"],
            vec![("doc:delete".into(), 0.52)],
        );
        let d = try_dialog_pairing(&s, &reg()).expect("mail pairing");
        assert_eq!(d.rule_id, "MAIL_DIALOG_PAIRING");
        assert_eq!(d.matched_tool_names()[0], "mail:delete");
    }

    #[test]
    fn calendar_cancel_after_list() {
        let s = signals(
            "please cancel that meeting on my calendar",
            &["calendar:list"],
            vec![("agenda:remove".into(), 0.55)],
        );
        let d = try_dialog_pairing(&s, &reg()).expect("calendar pairing");
        assert_eq!(d.rule_id, "CALENDAR_DIALOG_PAIRING");
        assert_eq!(d.matched_tool_names()[0], "calendar:delete");
        match &d.offer {
            RoutingOffer::DomainCluster { domains, .. } => {
                assert_eq!(domains.as_slice(), ["calendar"])
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bare_remove_after_doc_list_does_not_pair_doc() {
        let s = signals(
            "please remove it",
            &["doc:list"],
            vec![("doc:delete".into(), 0.51)],
        );
        assert!(try_dialog_pairing(&s, &reg()).is_none());
    }

    #[test]
    fn doc_anchored_delete_after_list() {
        let s = signals(
            "please delete that document from the ingested store",
            &["doc:list"],
            vec![("agenda:remove".into(), 0.52)],
        );
        let d = try_dialog_pairing(&s, &reg()).expect("doc pairing");
        assert_eq!(d.rule_id, "DOC_DIALOG_PAIRING");
        assert_eq!(d.matched_tool_names()[0], "doc:delete");
        // Prefer delete/list/query; ingest may remain in cluster but not first.
        assert_ne!(d.matched_tool_names()[0], "doc:ingest");
    }

    #[test]
    fn agenda_beats_mail_when_both_recent() {
        let s = signals(
            "please remove that from my agenda",
            &["mail:check", "agenda:list"],
            vec![("mail:delete".into(), 0.6)],
        );
        let d = try_dialog_pairing(&s, &reg()).expect("agenda first");
        assert_eq!(d.rule_id, "AGENDA_DIALOG_PAIRING");
    }
}
