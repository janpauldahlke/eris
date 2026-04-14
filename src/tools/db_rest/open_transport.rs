//! v6.db.transport.rest: `/locations` then `/journeys` via [`crate::util::ApiHttpClient`] profiles.

use std::collections::HashMap;

use chrono::DateTime;
use chrono::FixedOffset;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::executive::error::{FcpError, Result};
use crate::util::ApiHttpClient;

pub const PROFILE_LOCATIONS: &str = "db_rest_locations";
pub const PROFILE_JOURNEYS_DEPARTURE: &str = "db_rest_journeys_departure";
pub const PROFILE_JOURNEYS_ARRIVAL: &str = "db_rest_journeys_arrival";

pub const HINT_OUTPUT: &str = "Summarize each journey for the user: departure/arrival times (local), line names (e.g. ICE), platforms when present, delays from departureDelay/arrivalDelay (seconds). Mention transfers if multiple legs. Data is from a third-party timetable mirror; it may differ slightly from the official DB Navigator app.";

pub fn map_api_err(tool_name: &'static str, e: FcpError) -> FcpError {
    match e {
        FcpError::ToolFault { tool_name: tn, reason } if tn == "api_client" => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason,
        },
        FcpError::NetworkFault(_) => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "timetable service unreachable".into(),
        },
        FcpError::Config(msg) => FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: format!("timetable API configuration: {msg}"),
        },
        other => other,
    }
}

#[derive(Deserialize)]
struct LocationStop {
    id: Option<String>,
    name: Option<String>,
}

/// Parses the top-level JSON array from `/locations`; returns the first entry that has an `id`.
pub fn parse_first_stop_id(tool_name: &'static str, label: &str, body: &str) -> Result<(String, String)> {
    let rows: Vec<LocationStop> = serde_json::from_str(body).map_err(|e| FcpError::ToolFault {
        tool_name: tool_name.to_string(),
        reason: format!("locations JSON parse error for {label}: {e}"),
    })?;
    let hit = rows.iter().find(|r| r.id.as_deref().unwrap_or("").len() > 1).ok_or_else(|| {
        FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: format!("no station or stop match for {label}"),
        }
    })?;
    let id = hit.id.clone().ok_or_else(|| FcpError::ToolFault {
        tool_name: tool_name.to_string(),
        reason: format!("location hit missing id for {label}"),
    })?;
    let name = hit
        .name
        .clone()
        .unwrap_or_else(|| id.clone());
    Ok((id, name))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JourneysBody {
    journeys: Option<Vec<JourneyRaw>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JourneyRaw {
    legs: Vec<LegRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegRaw {
    origin: StopRef,
    destination: StopRef,
    departure: Option<String>,
    planned_departure: Option<String>,
    departure_delay: Option<i64>,
    arrival: Option<String>,
    planned_arrival: Option<String>,
    arrival_delay: Option<i64>,
    line: Option<LineRef>,
    direction: Option<String>,
    departure_platform: Option<String>,
    planned_departure_platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StopRef {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LineRef {
    name: Option<String>,
    mode: Option<String>,
    product: Option<String>,
}

fn normalize_journeys(tool_name: &'static str, body: &str, max_journeys: usize) -> Result<Vec<Value>> {
    let parsed: JourneysBody = serde_json::from_str(body).map_err(|e| FcpError::ToolFault {
        tool_name: tool_name.to_string(),
        reason: format!("journeys JSON parse error: {e}"),
    })?;
    let list = parsed.journeys.unwrap_or_default();
    if list.is_empty() {
        return Err(FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "no journeys returned for this origin, destination, and time".into(),
        });
    }
    let mut out = Vec::new();
    for j in list.into_iter().take(max_journeys) {
        let legs: Vec<Value> = j
            .legs
            .into_iter()
            .map(|leg| {
                json!({
                    "from": {
                        "id": leg.origin.id,
                        "name": leg.origin.name,
                    },
                    "to": {
                        "id": leg.destination.id,
                        "name": leg.destination.name,
                    },
                    "departure": leg.departure,
                    "plannedDeparture": leg.planned_departure,
                    "departureDelaySec": leg.departure_delay,
                    "arrival": leg.arrival,
                    "plannedArrival": leg.planned_arrival,
                    "arrivalDelaySec": leg.arrival_delay,
                    "line": leg.line.as_ref().map(|l| json!({
                        "name": l.name,
                        "mode": l.mode,
                        "product": l.product,
                    })),
                    "direction": leg.direction,
                    "departurePlatform": leg.departure_platform,
                    "plannedDeparturePlatform": leg.planned_departure_platform,
                })
            })
            .collect();
        out.push(json!({ "legs": legs }));
    }
    Ok(out)
}

/// Resolves `from` and `to` strings to stop ids, then fetches up to `max_journeys` connections.
pub async fn run_find_connections(
    api: &ApiHttpClient,
    tool_name: &'static str,
    from: &str,
    to: &str,
    when_rfc3339: &str,
    arrival_time: bool,
    max_journeys: usize,
) -> Result<String> {
    let mut p_from = HashMap::new();
    p_from.insert("query".into(), from.to_string());
    let loc_from = api
        .get_templated(PROFILE_LOCATIONS, &p_from)
        .await
        .map_err(|e| map_api_err(tool_name, e))?;
    let (id_from, name_from) = parse_first_stop_id(tool_name, "from", &loc_from)?;
    tracing::debug!(
        tool = tool_name,
        phase = "locations_from",
        query_char_len = from.chars().count(),
        stop_id = %id_from,
        stop_name = %name_from,
        "db:find_connections resolved origin stop"
    );

    let mut p_to = HashMap::new();
    p_to.insert("query".into(), to.to_string());
    let loc_to = api
        .get_templated(PROFILE_LOCATIONS, &p_to)
        .await
        .map_err(|e| map_api_err(tool_name, e))?;
    let (id_to, name_to) = parse_first_stop_id(tool_name, "to", &loc_to)?;
    tracing::debug!(
        tool = tool_name,
        phase = "locations_to",
        query_char_len = to.chars().count(),
        stop_id = %id_to,
        stop_name = %name_to,
        "db:find_connections resolved destination stop"
    );

    let profile = if arrival_time {
        PROFILE_JOURNEYS_ARRIVAL
    } else {
        PROFILE_JOURNEYS_DEPARTURE
    };
    let mut pj = HashMap::new();
    pj.insert("from".into(), id_from.clone());
    pj.insert("to".into(), id_to.clone());
    pj.insert("when".into(), when_rfc3339.to_string());
    let journey_body = api
        .get_templated(profile, &pj)
        .await
        .map_err(|e| map_api_err(tool_name, e))?;

    let journeys = normalize_journeys(tool_name, &journey_body, max_journeys)?;
    tracing::debug!(
        tool = tool_name,
        phase = "journeys",
        profile_id = profile,
        journey_count = journeys.len(),
        arrival_time_constraint = arrival_time,
        "db:find_connections fetched journey list"
    );

    let envelope = json!({
        "tool": tool_name,
        "resolvedFrom": { "id": id_from, "name": name_from, "query": from },
        "resolvedTo": { "id": id_to, "name": name_to, "query": to },
        "when": when_rfc3339,
        "timeConstraint": if arrival_time { "arrival" } else { "departure" },
        "hint": HINT_OUTPUT,
        "journeys": journeys,
    });
    serde_json::to_string(&envelope).map_err(FcpError::ParseFault)
}

/// Validates `when` is parseable as an offset datetime (ISO-8601 style).
pub fn validate_when_iso(when: &str) -> Result<()> {
    let t = when.trim();
    if t.is_empty() {
        return Err(FcpError::SchemaViolation(
            "`when` must be a non-empty ISO-8601 datetime with offset (e.g. 2026-04-15T08:00:00+02:00)".into(),
        ));
    }
    DateTime::parse_from_rfc3339(t)
        .or_else(|_| t.parse::<DateTime<FixedOffset>>())
        .map_err(|_| {
            FcpError::SchemaViolation(
                "`when` must include a timezone offset, e.g. 2026-04-15T08:00:00+02:00 (RFC 3339)".into(),
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_first_stop_ok() {
        let body = r#"[{"type":"stop","id":"8002549","name":"Hamburg Hbf"}]"#;
        let (id, name) = parse_first_stop_id("db:find_connections", "from", body).expect("ok");
        assert_eq!(id, "8002549");
        assert_eq!(name, "Hamburg Hbf");
    }

    #[test]
    fn parse_first_stop_empty() {
        let body = r#"[]"#;
        let r = parse_first_stop_id("db:find_connections", "from", body);
        assert!(matches!(r, Err(FcpError::ToolFault { .. })));
    }

    #[test]
    fn normalize_one_journey() {
        let body = r#"{"journeys":[{"legs":[{"origin":{"id":"1","name":"A"},"destination":{"id":"2","name":"B"},"departure":"2026-04-15T08:00:00+02:00","plannedDeparture":"2026-04-15T08:00:00+02:00","departureDelay":0,"arrival":"2026-04-15T09:00:00+02:00","plannedArrival":"2026-04-15T09:00:00+02:00","arrivalDelay":60,"line":{"name":"ICE 1","mode":"train"},"direction":"München","departurePlatform":"1","plannedDeparturePlatform":"1"}]}]}"#;
        let v = normalize_journeys("db:find_connections", body, 3).expect("ok");
        assert_eq!(v.len(), 1);
        assert!(v[0].get("legs").and_then(|l| l.as_array()).is_some());
    }

    #[test]
    fn validate_when_accepts_offset() {
        validate_when_iso("2026-04-15T08:00:00+02:00").expect("ok");
    }

    #[test]
    fn validate_when_rejects_naive() {
        let r = validate_when_iso("2026-04-15T08:00:00");
        assert!(matches!(r, Err(FcpError::SchemaViolation(_))));
    }
}
