//! Deterministic Open-Meteo → human weather reports for the LLM (no raw JSON interpretation).

use serde_json::Value;

use crate::executive::error::{FcpError, Result};

pub fn wmo_emoji(code: i64) -> &'static str {
    match code {
        0 => "☀️",
        1 => "🌤️",
        2 => "⛅",
        3 => "☁️",
        45 | 48 => "🌫️",
        51 | 53 | 55 | 56 | 57 => "🌦️",
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => "🌧️",
        71 | 73 | 75 | 77 | 85 | 86 => "❄️",
        95 | 96 | 99 => "⛈️",
        _ => "🌡️",
    }
}

/// WMO weather interpretation codes (Open-Meteo / WMO WW).
pub fn wmo_label(code: i64) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Depositing rime fog",
        51 => "Light drizzle",
        53 => "Moderate drizzle",
        55 => "Dense drizzle",
        56 => "Light freezing drizzle",
        57 => "Dense freezing drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        66 => "Light freezing rain",
        67 => "Heavy freezing rain",
        71 => "Slight snow",
        73 => "Moderate snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Slight rain showers",
        81 => "Moderate rain showers",
        82 => "Violent rain showers",
        85 => "Slight snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 => "Thunderstorm with slight hail",
        99 => "Thunderstorm with heavy hail",
        _ => "Unknown conditions",
    }
}

pub fn cloud_label(pct: f64) -> &'static str {
    if pct <= 20.0 {
        "mostly clear"
    } else if pct <= 50.0 {
        "partly cloudy"
    } else if pct <= 80.0 {
        "mostly cloudy"
    } else {
        "overcast"
    }
}

pub fn precip_label(mm: f64) -> &'static str {
    if mm <= 0.0 {
        "none"
    } else if mm < 0.2 {
        "trace"
    } else if mm < 2.0 {
        "light"
    } else if mm < 10.0 {
        "moderate"
    } else {
        "heavy"
    }
}

pub fn heat_note(c: f64) -> Option<&'static str> {
    if c >= 35.0 {
        Some("extreme heat 🥵")
    } else if c >= 30.0 {
        Some("very hot 🥵")
    } else if c <= -10.0 {
        Some("extreme cold 🥶")
    } else if c <= 0.0 {
        Some("freezing 🥶")
    } else {
        None
    }
}

pub fn uv_label(index: f64) -> &'static str {
    if index < 3.0 {
        "low"
    } else if index < 6.0 {
        "moderate"
    } else if index < 8.0 {
        "high"
    } else if index < 11.0 {
        "very high"
    } else {
        "extreme"
    }
}

fn wind_compass(degrees: f64) -> &'static str {
    let d = degrees.rem_euclid(360.0);
    match d {
        x if x < 22.5 => "N",
        x if x < 67.5 => "NE",
        x if x < 112.5 => "E",
        x if x < 157.5 => "SE",
        x if x < 202.5 => "S",
        x if x < 247.5 => "SW",
        x if x < 292.5 => "W",
        x if x < 337.5 => "NW",
        _ => "N",
    }
}

fn format_wind(speed_kmh: f64, direction_deg: Option<f64>) -> String {
    match direction_deg {
        Some(deg) => format!(
            "💨 Wind: {:.0} km/h from {} ({:.0}°)",
            speed_kmh,
            wind_compass(deg),
            deg
        ),
        None => format!("💨 Wind: {:.0} km/h", speed_kmh),
    }
}

fn format_uv(index: f64) -> String {
    format!("🌞 UV index: {:.1} ({})", index, uv_label(index))
}

fn format_temp(c: f64) -> String {
    format!("{:.1} °C", c)
}

fn f64_field(obj: &Value, key: &str) -> Option<f64> {
    obj.get(key).and_then(|v| v.as_f64())
}

fn i64_field(obj: &Value, key: &str) -> Option<i64> {
    obj.get(key).and_then(|v| v.as_i64())
}

fn timezone_line(data: &Value) -> String {
    let tz = data
        .get("timezone")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let abbrev = data
        .get("timezone_abbreviation")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match abbrev {
        Some(a) => format!("{tz} ({a})"),
        None => tz.to_string(),
    }
}

fn precip_interval_label(interval_secs: Option<i64>) -> &'static str {
    match interval_secs {
        Some(900) => "last 15 min",
        Some(3600) => "last hour",
        Some(s) if s > 0 => "recent interval",
        _ => "last 15 min",
    }
}

pub fn format_current_report(location: &str, data: &Value) -> Result<String> {
    let current = data.get("current").ok_or_else(|| FcpError::ToolFault {
        tool_name: "weather:current".into(),
        reason: "forecast response missing `current` block".into(),
    })?;
    let temp = f64_field(current, "temperature_2m").ok_or_else(|| FcpError::ToolFault {
        tool_name: "weather:current".into(),
        reason: "current block missing temperature_2m".into(),
    })?;
    let observed = current
        .get("time")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let interval = i64_field(current, "interval");
    let precip_key = precip_interval_label(interval);

    let mut temp_line = format!("**{}**", format_temp(temp));
    if let Some(feels) = f64_field(current, "apparent_temperature") {
        temp_line.push_str(&format!(" · feels **{}**", format_temp(feels)));
        if let Some(note) = heat_note(feels.max(temp)) {
            temp_line.push_str(&format!(" · {note}"));
        }
    } else if let Some(note) = heat_note(temp) {
        temp_line.push_str(&format!(" · {note}"));
    }

    let mut out = format!("## 🌡️ Now · {location}\n\n");
    out.push_str(&format!(
        "_{}_ · observed {}\n\n",
        timezone_line(data),
        observed
    ));

    if let Some(code) = i64_field(current, "weather_code") {
        out.push_str(&format!(
            "{} {} · {}\n\n",
            wmo_emoji(code),
            temp_line,
            wmo_label(code)
        ));
    } else {
        out.push_str(&format!("{temp_line}\n\n"));
    }

    out.push_str("### Details\n");
    if let Some(h) = f64_field(current, "relative_humidity_2m") {
        out.push_str(&format!("- 💧 Humidity **{:.0}%**\n", h));
    }
    if let Some(p) = f64_field(current, "precipitation") {
        let rain_emoji = if p > 0.0 { "🌧️" } else { "💧" };
        out.push_str(&format!(
            "- {rain_emoji} Rain ({precip_key}) **{:.1} mm** · {}\n",
            p,
            precip_label(p)
        ));
    }
    if let Some(cc) = f64_field(current, "cloud_cover") {
        out.push_str(&format!(
            "- ☁️ Clouds **{:.0}%** · {}\n",
            cc,
            cloud_label(cc)
        ));
    }
    if let Some(ws) = f64_field(current, "wind_speed_10m") {
        let wd = f64_field(current, "wind_direction_10m");
        let wind = format_wind(ws, wd);
        out.push_str(&format!("- {wind}\n"));
    }
    if let Some(uv) = f64_field(current, "uv_index") {
        out.push_str(&format!("- {}\n", format_uv(uv)));
    }
    while out.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

fn f64_array<'a>(obj: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    obj.get(key).and_then(|v| v.as_array())
}

fn str_array<'a>(obj: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    obj.get(key).and_then(|v| v.as_array())
}

fn value_as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn value_as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
}

fn hour_from_iso(time: &str) -> Option<String> {
    // Open-Meteo local times: "2026-06-24T17:45" or "2026-06-24T17:00"
    let t = time.split('T').nth(1)?;
    Some(t[..std::cmp::min(5, t.len())].to_string())
}

fn day_month_label(date: &str) -> String {
    use chrono::NaiveDate;
    match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => d.format("%a %d %b").to_string(),
        Err(_) => date.to_string(),
    }
}

/// Pull the formatted `report` field from a weather tool JSON envelope.
pub fn report_from_tool_envelope(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.get("report")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string())
}

/// Merge one or more weather tool reports for direct display in the chat deck.
pub fn compose_weather_deck_message(parts: &[(&str, String)]) -> String {
    parts
        .iter()
        .map(|(_, report)| report.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn dominant_weather_code(codes: &[i64]) -> Option<i64> {
    if codes.is_empty() {
        return None;
    }
    // Prefer the most "severe" code in the bucket (higher code often = worse).
    codes.iter().copied().max()
}

fn format_next_24h(data: &Value, out: &mut String) -> Result<()> {
    let hourly = data.get("hourly").ok_or_else(|| FcpError::ToolFault {
        tool_name: "weather:forecast".into(),
        reason: "forecast response missing `hourly` block".into(),
    })?;
    let times = str_array(hourly, "time").ok_or_else(|| FcpError::ToolFault {
        tool_name: "weather:forecast".into(),
        reason: "hourly block missing time array".into(),
    })?;
    let temps = f64_array(hourly, "temperature_2m");
    let precips = f64_array(hourly, "precipitation");
    let codes = hourly.get("weather_code").and_then(|v| v.as_array());
    let is_days = hourly.get("is_day").and_then(|v| v.as_array());
    let winds = f64_array(hourly, "wind_speed_10m");
    let wind_dirs = f64_array(hourly, "wind_direction_10m");
    let rain_probs = f64_array(hourly, "precipitation_probability");
    let uvs = f64_array(hourly, "uv_index");

    let anchor = data
        .get("current")
        .and_then(|c| c.get("time"))
        .and_then(|v| v.as_str())
        .or_else(|| times.first().and_then(|v| v.as_str()))
        .unwrap_or("");

    let start_idx = times
        .iter()
        .position(|t| t.as_str().map(|s| s >= anchor).unwrap_or(false))
        .unwrap_or(0);

    out.push_str("\n### ⏳ Next 24 hours\n");

    const BUCKET_COUNT: usize = 3;
    const BUCKET_HOURS: usize = 8;
    const TOTAL_HOURS: usize = 24;
    let mut h = 0usize;
    let mut idx = start_idx;
    let mut bucket_num = 0usize;

    while h < TOTAL_HOURS && idx < times.len() && bucket_num < BUCKET_COUNT {
        let mut end_idx = idx;
        let mut hours_in_bucket = 0usize;
        while hours_in_bucket < BUCKET_HOURS && end_idx < times.len() && h + hours_in_bucket < TOTAL_HOURS
        {
            hours_in_bucket += 1;
            end_idx += 1;
        }
        if hours_in_bucket == 0 {
            break;
        }

        let slice_end = end_idx;
        let time_start = times[idx].as_str().unwrap_or("");
        let time_end_idx = slice_end.saturating_sub(1);
        let time_end = times
            .get(time_end_idx)
            .and_then(|v| v.as_str())
            .unwrap_or(time_start);

        let start_h = hour_from_iso(time_start).unwrap_or_else(|| "?".into());
        let end_h = hour_from_iso(time_end).unwrap_or_else(|| "?".into());

        let mut t_sum = 0.0f64;
        let mut t_n = 0usize;
        let mut precip_sum = 0.0f64;
        let mut code_list = Vec::new();
        let mut any_day = false;
        let mut any_night = false;
        let mut wind_sum = 0.0f64;
        let mut wind_n = 0usize;
        let mut wind_dir_sum = 0.0f64;
        let mut wind_dir_n = 0usize;
        let mut rain_prob_max = 0.0f64;
        let mut uv_sum = 0.0f64;
        let mut uv_n = 0usize;

        for i in idx..slice_end {
            if let Some(arr) = temps {
                if let Some(t) = arr.get(i).and_then(value_as_f64) {
                    t_sum += t;
                    t_n += 1;
                }
            }
            if let Some(arr) = precips {
                if let Some(p) = arr.get(i).and_then(value_as_f64) {
                    precip_sum += p;
                }
            }
            if let Some(arr) = codes {
                if let Some(c) = arr.get(i).and_then(value_as_i64) {
                    code_list.push(c);
                }
            }
            if let Some(arr) = is_days {
                match arr.get(i).and_then(value_as_i64) {
                    Some(1) => any_day = true,
                    Some(0) => any_night = true,
                    _ => {}
                }
            }
            if let Some(arr) = winds {
                if let Some(w) = arr.get(i).and_then(value_as_f64) {
                    wind_sum += w;
                    wind_n += 1;
                }
            }
            if let Some(arr) = wind_dirs {
                if let Some(d) = arr.get(i).and_then(value_as_f64) {
                    wind_dir_sum += d;
                    wind_dir_n += 1;
                }
            }
            if let Some(arr) = rain_probs {
                if let Some(p) = arr.get(i).and_then(value_as_f64) {
                    rain_prob_max = rain_prob_max.max(p);
                }
            }
            if let Some(arr) = uvs {
                if let Some(u) = arr.get(i).and_then(value_as_f64) {
                    uv_sum += u;
                    uv_n += 1;
                }
            }
        }

        let temp_part = if t_n > 0 {
            format!("avg **{:.0} °C**", t_sum / t_n as f64)
        } else {
            "avg n/a".into()
        };

        let dominant = dominant_weather_code(&code_list);
        let emoji = dominant.map(wmo_emoji).unwrap_or("🌡️");
        let cond = dominant.map(wmo_label).unwrap_or("n/a");

        let wind_part = if wind_n > 0 {
            let wind_avg = wind_sum / wind_n as f64;
            let avg_dir = if wind_dir_n > 0 {
                Some(wind_dir_sum / wind_dir_n as f64)
            } else {
                None
            };
            match avg_dir {
                Some(deg) => format!(", 💨 {:.0} km/h {}", wind_avg, wind_compass(deg)),
                None => format!(", 💨 {:.0} km/h", wind_avg),
            }
        } else {
            String::new()
        };

        let rain_part = if precip_sum > 0.0 {
            format!(
                ", 🌧️ {:.1} mm ({}% chance)",
                precip_sum,
                rain_prob_max.round()
            )
        } else if rain_prob_max > 0.0 {
            format!(", 🌦️ {:.0}% rain chance", rain_prob_max.round())
        } else {
            ", ☀️ dry".to_string()
        };

        let uv_part = if uv_n > 0 {
            let uv_avg = uv_sum / uv_n as f64;
            format!(", 🌞 UV {:.0} ({})", uv_avg.round(), uv_label(uv_avg))
        } else {
            String::new()
        };

        let day_part = if any_day && any_night {
            ", 🌓 day/night"
        } else if any_day {
            ", ☀️ day"
        } else if any_night {
            ", 🌙 night"
        } else {
            ""
        };

        out.push_str(&format!(
            "- **{start_h}–{end_h}** {emoji} {temp_part}, {cond}{rain_part}{wind_part}{uv_part}{day_part}\n"
        ));

        h += hours_in_bucket;
        idx = slice_end;
        bucket_num += 1;
    }

    if bucket_num == 0 {
        out.push_str("- (no hourly data available)\n");
    }
    Ok(())
}

fn format_current_snapshot(data: &Value, out: &mut String) {
    let Some(current) = data.get("current") else {
        return;
    };
    out.push_str("\n\n");
    if let (Some(t), Some(code)) = (
        f64_field(current, "temperature_2m"),
        i64_field(current, "weather_code"),
    ) {
        let mut line = format!(
            "**Right now** {} **{}** · {}",
            wmo_emoji(code),
            format_temp(t),
            wmo_label(code)
        );
        if let Some(note) = heat_note(t) {
            line.push_str(&format!(" · {note}"));
        }
        out.push_str(&line);
    } else if let Some(t) = f64_field(current, "temperature_2m") {
        out.push_str(&format!("**Right now** **{}**", format_temp(t)));
    }
    let mut extras = Vec::new();
    if let Some(ws) = f64_field(current, "wind_speed_10m") {
        let wd = f64_field(current, "wind_direction_10m");
        extras.push(format_wind(ws, wd));
    }
    if let Some(uv) = f64_field(current, "uv_index") {
        extras.push(format_uv(uv));
    }
    if !extras.is_empty() {
        out.push_str("\n");
        out.push_str(&extras.join(" · "));
    }
}

fn format_daily_outlook(data: &Value, out: &mut String) {
    let Some(daily) = data.get("daily") else {
        out.push_str("\nDaily outlook: (daily block not available in API response)\n");
        return;
    };
    let Some(times) = str_array(daily, "time") else {
        out.push_str("\nDaily outlook: (daily time array missing)\n");
        return;
    };
    let mins = f64_array(daily, "temperature_2m_min");
    let maxs = f64_array(daily, "temperature_2m_max");
    let precips = f64_array(daily, "precipitation_sum");
    let codes = daily.get("weather_code").and_then(|v| v.as_array());
    let rain_probs = f64_array(daily, "precipitation_probability_max");
    let uv_maxs = f64_array(daily, "uv_index_max");
    let wind_maxs = f64_array(daily, "wind_speed_10m_max");
    let gust_maxs = f64_array(daily, "wind_gusts_10m_max");

    out.push_str("\n### 📅 Next few days\n");
    for (i, t) in times.iter().enumerate() {
        let date = t.as_str().unwrap_or("");
        let day_label = day_month_label(date);
        let min_t = mins
            .and_then(|a| a.get(i))
            .and_then(value_as_f64)
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "?".into());
        let max_t = maxs
            .and_then(|a| a.get(i))
            .and_then(value_as_f64)
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "?".into());
        let precip = precips
            .and_then(|a| a.get(i))
            .and_then(value_as_f64)
            .unwrap_or(0.0);
        let code = codes.and_then(|a| a.get(i)).and_then(value_as_i64);
        let emoji = code.map(wmo_emoji).unwrap_or("🌡️");
        let cond = code.map(wmo_label).unwrap_or("n/a");
        let rain_prob = rain_probs
            .and_then(|a| a.get(i))
            .and_then(value_as_f64);
        let rain_part = match (precip, rain_prob) {
            (p, Some(prob)) if p > 0.0 || prob > 0.0 => {
                format!("🌧️ {:.1} mm (up to {:.0}% chance)", p, prob)
            }
            (_, Some(prob)) if prob > 0.0 => format!("🌦️ {:.0}% rain chance", prob),
            (p, _) if p > 0.0 => format!("🌧️ {:.1} mm", p),
            _ => "☀️ dry".to_string(),
        };
        let wind_part = match (
            wind_maxs.and_then(|a| a.get(i)).and_then(value_as_f64),
            gust_maxs.and_then(|a| a.get(i)).and_then(value_as_f64),
        ) {
            (Some(w), Some(g)) => format!("💨 **{:.0}** km/h · gusts **{:.0}**", w, g),
            (Some(w), None) => format!("💨 up to **{:.0}** km/h", w),
            _ => String::new(),
        };
        let uv_part = uv_maxs
            .and_then(|a| a.get(i))
            .and_then(value_as_f64)
            .map(|u| format!("🌞 UV **{:.0}** ({})", u.round(), uv_label(u)))
            .unwrap_or_default();

        let temp_span = format!("**{min_t}–{max_t} °C**");
        let extras = [wind_part, uv_part]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        if extras.is_empty() {
            out.push_str(&format!(
                "- **{day_label}** {emoji} {temp_span} · {rain_part} · {cond}\n"
            ));
        } else {
            out.push_str(&format!(
                "- **{day_label}** {emoji} {temp_span} · {rain_part} · {extras} · {cond}\n"
            ));
        }
    }
}

pub fn format_forecast_report(location: &str, data: &Value) -> Result<String> {
    let as_of = data
        .get("current")
        .and_then(|c| c.get("time"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            data.get("hourly")
                .and_then(|h| h.get("time"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown");

    let mut out = format!("## 🌤️ Forecast · {location}\n\n");
    out.push_str(&format!("_{}_ · as of {as_of}_", timezone_line(data)));
    format_current_snapshot(data, &mut out);

    format_next_24h(data, &mut out)?;
    format_daily_outlook(data, &mut out);

    while out.ends_with('\n') {
        out.pop();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wmo_label_known_codes() {
        assert_eq!(wmo_label(0), "Clear sky");
        assert_eq!(wmo_label(2), "Partly cloudy");
        assert_eq!(wmo_label(95), "Thunderstorm");
    }

    #[test]
    fn wmo_emoji_known_codes() {
        assert_eq!(wmo_emoji(0), "☀️");
        assert_eq!(wmo_emoji(61), "🌧️");
        assert_eq!(wmo_emoji(95), "⛈️");
    }

    #[test]
    fn heat_note_thresholds() {
        assert_eq!(heat_note(32.0), Some("very hot 🥵"));
        assert_eq!(heat_note(36.0), Some("extreme heat 🥵"));
        assert_eq!(heat_note(20.0), None);
    }

    #[test]
    fn uv_label_bands() {
        assert_eq!(uv_label(2.0), "low");
        assert_eq!(uv_label(5.0), "moderate");
        assert_eq!(uv_label(9.0), "very high");
    }

    #[test]
    fn format_current_report_hamburg_fixture() {
        let data = json!({
            "timezone": "Europe/Berlin",
            "timezone_abbreviation": "GMT+2",
            "current": {
                "time": "2026-06-24T17:45",
                "interval": 900,
                "temperature_2m": 32.4,
                "apparent_temperature": 33.0,
                "weather_code": 2,
                "relative_humidity_2m": 36,
                "precipitation": 0.0,
                "cloud_cover": 41,
                "wind_speed_10m": 12.5,
                "wind_direction_10m": 315.0,
                "uv_index": 4.2
            }
        });
        let report = format_current_report("Hamburg, Germany", &data).expect("report");
        assert!(report.contains("## 🌡️ Now · Hamburg, Germany"));
        assert!(report.contains("**32.4 °C**"));
        assert!(report.contains("very hot 🥵"));
        assert!(report.contains("⛅"));
        assert!(report.contains("Partly cloudy"));
        assert!(report.contains("💨"));
        assert!(report.contains("🌞 UV"));
        assert!(report.contains("### Details"));
    }

    #[test]
    fn format_forecast_report_daily_and_buckets() {
        let data = json!({
            "timezone": "Europe/Berlin",
            "timezone_abbreviation": "CEST",
            "current": {
                "time": "2026-06-24T17:00",
                "temperature_2m": 32.0,
                "weather_code": 2,
                "wind_speed_10m": 10.0,
                "wind_direction_10m": 270.0,
                "uv_index": 5.0
            },
            "hourly": {
                "time": [
                    "2026-06-24T17:00","2026-06-24T18:00","2026-06-24T19:00",
                    "2026-06-24T20:00","2026-06-24T21:00","2026-06-24T22:00",
                    "2026-06-24T23:00","2026-06-25T00:00","2026-06-25T01:00",
                    "2026-06-25T02:00","2026-06-25T03:00","2026-06-25T04:00",
                    "2026-06-25T05:00","2026-06-25T06:00","2026-06-25T07:00",
                    "2026-06-25T08:00","2026-06-25T09:00","2026-06-25T10:00",
                    "2026-06-25T11:00","2026-06-25T12:00","2026-06-25T13:00",
                    "2026-06-25T14:00","2026-06-25T15:00","2026-06-25T16:00",
                    "2026-06-25T17:00"
                ],
                "temperature_2m": [32.0,31.0,30.0,29.0,28.0,27.0,26.0,25.0,24.0,23.0,22.0,21.0,20.0,21.0,22.0,23.0,24.0,25.0,26.0,27.0,28.0,29.0,30.0,31.0,32.0],
                "precipitation": [0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0],
                "weather_code": [2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2],
                "is_day": [1,1,1,1,1,1,0,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1,1,1,1],
                "wind_speed_10m": [10.0,11.0,12.0,10.0,9.0,8.0,7.0,6.0,5.0,5.0,4.0,4.0,5.0,6.0,7.0,8.0,9.0,10.0,11.0,12.0,13.0,14.0,15.0,14.0,13.0],
                "wind_direction_10m": [270.0,275.0,280.0,270.0,265.0,260.0,255.0,250.0,245.0,240.0,235.0,230.0,225.0,220.0,225.0,230.0,235.0,240.0,245.0,250.0,255.0,260.0,265.0,270.0,275.0],
                "precipitation_probability": [5.0,5.0,10.0,10.0,15.0,20.0,25.0,30.0,20.0,15.0,10.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0,5.0],
                "uv_index": [5.0,4.0,3.0,2.0,1.0,0.5,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.5,1.0,2.0,3.0,4.0,5.0,6.0,7.0,6.0,5.0,4.0,3.0]
            },
            "daily": {
                "time": ["2026-06-24","2026-06-25","2026-06-26"],
                "temperature_2m_min": [17.0, 20.0, 18.0],
                "temperature_2m_max": [32.0, 29.0, 36.0],
                "precipitation_sum": [0.0, 1.4, 0.0],
                "weather_code": [2, 3, 0],
                "precipitation_probability_max": [10.0, 60.0, 5.0],
                "uv_index_max": [7.0, 5.0, 9.0],
                "wind_speed_10m_max": [15.0, 18.0, 12.0],
                "wind_gusts_10m_max": [25.0, 30.0, 20.0]
            }
        });
        let report = format_forecast_report("Hamburg, Germany", &data).expect("report");
        assert!(report.contains("## 🌤️ Forecast · Hamburg, Germany"));
        assert!(report.contains("**Right now**"));
        assert!(report.contains("💨"));
        assert!(report.contains("### ⏳ Next 24 hours"));
        assert!(report.contains("avg **"));
        assert!(report.contains("🌦️"));
        assert!(report.contains("### 📅 Next few days"));
        assert!(report.contains("🌞 UV **9**"));
        assert!(report.contains("36"));
        assert!(report.contains("17:00"));
    }
}
