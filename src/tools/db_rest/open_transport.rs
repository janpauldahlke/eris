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

pub const HINT_OUTPUT: &str = "Each journey has `summary` (door-to-door depart/arrive, durationMinutes, transferCount, direct), `rides` (each train: line, operator, from/to stop names, depart/arrive RFC3339, depPlatform/arrPlatform, depDelaySec/arrDelaySec), and `transfers` (where to change and minutesBetween consecutive rides). Omit walking-only rows; they are folded into transfers. Data is a third-party timetable mirror; it may differ slightly from the official DB Navigator app.";

/// Upper bound on the serialized tool JSON so chat/context stays bounded.
const MAX_TOOL_RESULT_CHARS: usize = 8000;

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
    #[serde(default)]
    price: Option<PriceRaw>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceRaw {
    amount: Option<f64>,
    currency: Option<String>,
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
    arrival_platform: Option<String>,
    planned_arrival_platform: Option<String>,
    #[serde(default)]
    walking: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct StopRef {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LineRef {
    name: Option<String>,
    mode: Option<String>,
    product: Option<String>,
    #[serde(default)]
    operator: Option<OperatorRef>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperatorRef {
    name: Option<String>,
}

fn same_stop(a: &StopRef, b: &StopRef) -> bool {
    match (a.id.as_deref(), b.id.as_deref()) {
        (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() => x == y,
        _ => a.name == b.name && a.name.is_some(),
    }
}

/// In-station walking / placeholder leg the API inserts between trains.
fn is_transfer_placeholder(leg: &LegRaw) -> bool {
    if leg.walking == Some(true) {
        return true;
    }
    if leg.line.is_some() {
        return false;
    }
    same_stop(&leg.origin, &leg.destination)
}

fn pick_dt(actual: &Option<String>, planned: &Option<String>) -> Option<String> {
    if let Some(s) = actual {
        if !s.is_empty() {
            return Some(s.clone());
        }
    }
    planned.clone()
}

fn parse_rfc3339(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

fn fold_journey(j: JourneyRaw) -> Value {
    let motor_legs: Vec<LegRaw> = j.legs.into_iter().filter(|l| !is_transfer_placeholder(l)).collect();

    let rides: Vec<Value> = motor_legs
        .iter()
        .map(|leg| {
            let depart = pick_dt(&leg.departure, &leg.planned_departure);
            let arrive = pick_dt(&leg.arrival, &leg.planned_arrival);
            let dep_platform = leg
                .departure_platform
                .clone()
                .or_else(|| leg.planned_departure_platform.clone());
            let arr_platform = leg
                .arrival_platform
                .clone()
                .or_else(|| leg.planned_arrival_platform.clone());
            let operator = leg
                .line
                .as_ref()
                .and_then(|l| l.operator.as_ref())
                .and_then(|o| o.name.clone());
            json!({
                "line": leg.line.as_ref().and_then(|l| l.name.clone()),
                "mode": leg.line.as_ref().and_then(|l| l.mode.clone()),
                "product": leg.line.as_ref().and_then(|l| l.product.clone()),
                "operator": operator,
                "direction": leg.direction.clone(),
                "from": leg.origin.name.clone().or_else(|| leg.origin.id.clone()),
                "to": leg.destination.name.clone().or_else(|| leg.destination.id.clone()),
                "depart": depart,
                "arrive": arrive,
                "depDelaySec": leg.departure_delay,
                "arrDelaySec": leg.arrival_delay,
                "depPlatform": dep_platform,
                "arrPlatform": arr_platform,
            })
        })
        .collect();

    let mut transfers: Vec<Value> = Vec::new();
    for pair in motor_legs.windows(2) {
        let prev = &pair[0];
        let next = &pair[1];
        let arrive_s = pick_dt(&prev.arrival, &prev.planned_arrival);
        let depart_s = pick_dt(&next.departure, &next.planned_departure);
        let minutes_between = match (arrive_s.as_deref(), depart_s.as_deref()) {
            (Some(a), Some(d)) => match (parse_rfc3339(a), parse_rfc3339(d)) {
                (Some(ta), Some(td)) => Some(td.signed_duration_since(ta).num_minutes()),
                _ => None,
            },
            _ => None,
        };
        let at = prev
            .destination
            .name
            .clone()
            .or_else(|| prev.destination.id.clone())
            .unwrap_or_default();
        transfers.push(json!({
            "at": at,
            "minutesBetween": minutes_between,
        }));
    }

    let (duration_minutes, summary_depart, summary_arrive) = if let (Some(first), Some(last)) =
        (motor_legs.first(), motor_legs.last())
    {
        let d0 = pick_dt(&first.departure, &first.planned_departure);
        let a1 = pick_dt(&last.arrival, &last.planned_arrival);
        let dur = match (d0.as_deref(), a1.as_deref()) {
            (Some(ds), Some(as_)) => match (parse_rfc3339(ds), parse_rfc3339(as_)) {
                (Some(td), Some(ta)) => Some(ta.signed_duration_since(td).num_minutes()),
                _ => None,
            },
            _ => None,
        };
        (dur, d0, a1)
    } else {
        (None, None, None)
    };

    let transfer_count = motor_legs.len().saturating_sub(1);
    let direct = motor_legs.len() <= 1;

    let mut out = json!({
        "summary": {
            "depart": summary_depart,
            "arrive": summary_arrive,
            "durationMinutes": duration_minutes,
            "transferCount": transfer_count,
            "direct": direct,
        },
        "rides": rides,
        "transfers": transfers,
    });

    if let Some(p) = j.price {
        if p.amount.is_some() || p.currency.is_some() {
            if let Some(m) = out.as_object_mut() {
                m.insert(
                    "price".into(),
                    json!({
                        "amount": p.amount,
                        "currency": p.currency,
                    }),
                );
            }
        }
    }

    out
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
    let mut out: Vec<Value> = Vec::new();
    for j in list.into_iter().take(max_journeys) {
        let has_motor = j.legs.iter().any(|l| !is_transfer_placeholder(l));
        if has_motor {
            out.push(fold_journey(j));
        }
    }
    if out.is_empty() {
        return Err(FcpError::ToolFault {
            tool_name: tool_name.to_string(),
            reason: "no ride legs after folding transfers (unexpected API shape)".into(),
        });
    }
    Ok(out)
}

/// Shrinks `journeys` until serialized `envelope` fits `MAX_TOOL_RESULT_CHARS`, or returns a minimal error payload.
fn cap_envelope_json(mut envelope: Value) -> Result<String> {
    for _ in 0..32 {
        let s = serde_json::to_string(&envelope).map_err(FcpError::ParseFault)?;
        if s.len() <= MAX_TOOL_RESULT_CHARS {
            return Ok(s);
        }
        let Some(arr) = envelope.get_mut("journeys").and_then(|j| j.as_array_mut()) else {
            return Ok(s);
        };
        if arr.is_empty() {
            break;
        }
        arr.pop();
    }
    serde_json::to_string(&json!({
        "tool": envelope.get("tool").cloned().unwrap_or(json!("db:find_connections")),
        "error": "tool_result_too_large",
        "hint": HINT_OUTPUT,
    }))
    .map_err(FcpError::ParseFault)
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
    cap_envelope_json(envelope)
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
    fn normalize_one_journey_folded() {
        let body = r#"{"journeys":[{"legs":[{"origin":{"id":"1","name":"A"},"destination":{"id":"2","name":"B"},"departure":"2026-04-15T08:00:00+02:00","plannedDeparture":"2026-04-15T08:00:00+02:00","departureDelay":0,"arrival":"2026-04-15T09:00:00+02:00","plannedArrival":"2026-04-15T09:00:00+02:00","arrivalDelay":60,"line":{"name":"ICE 1","mode":"train"},"direction":"München","departurePlatform":"1","plannedDeparturePlatform":"1"}]}]}"#;
        let v = normalize_journeys("db:find_connections", body, 3).expect("ok");
        assert_eq!(v.len(), 1);
        let rides = v[0].get("rides").and_then(|r| r.as_array()).expect("rides");
        assert_eq!(rides.len(), 1);
        assert_eq!(rides[0].get("line").and_then(|x| x.as_str()), Some("ICE 1"));
        let summary = v[0].get("summary").expect("summary");
        assert_eq!(summary.get("direct").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(summary.get("transferCount").and_then(|x| x.as_u64()), Some(0));
    }

    #[test]
    fn normalize_folds_walking_transfer() {
        let body = r#"{"journeys":[{"legs":[
            {"origin":{"id":"1","name":"X"},"destination":{"id":"2","name":"Y"},"departure":"2026-04-15T08:00:00+02:00","plannedDeparture":"2026-04-15T08:00:00+02:00","arrival":"2026-04-15T09:00:00+02:00","plannedArrival":"2026-04-15T09:00:00+02:00","line":{"name":"ICE 9","mode":"train"}},
            {"origin":{"id":"2","name":"Y"},"destination":{"id":"2","name":"Y"},"departure":"2026-04-15T09:00:00+02:00","plannedDeparture":"2026-04-15T09:00:00+02:00","arrival":"2026-04-15T09:00:00+02:00","plannedArrival":"2026-04-15T09:00:00+02:00","walking":true},
            {"origin":{"id":"2","name":"Y"},"destination":{"id":"3","name":"Z"},"departure":"2026-04-15T09:30:00+02:00","plannedDeparture":"2026-04-15T09:30:00+02:00","arrival":"2026-04-15T10:00:00+02:00","plannedArrival":"2026-04-15T10:00:00+02:00","line":{"name":"RE 1","mode":"train"}}
        ]}]}"#;
        let v = normalize_journeys("db:find_connections", body, 3).expect("ok");
        let rides = v[0].get("rides").and_then(|r| r.as_array()).expect("rides");
        assert_eq!(rides.len(), 2);
        let transfers = v[0].get("transfers").and_then(|t| t.as_array()).expect("transfers");
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].get("at").and_then(|x| x.as_str()), Some("Y"));
        assert_eq!(transfers[0].get("minutesBetween").and_then(|x| x.as_i64()), Some(30));
        assert_eq!(v[0].get("summary").and_then(|s| s.get("direct")).and_then(|x| x.as_bool()), Some(false));
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
