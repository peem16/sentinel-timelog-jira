use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde::Serialize;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize)]
pub struct CalEvent {
    pub summary: String,
    /// local start, RFC3339
    pub start: String,
    pub end: String,
    pub duration_secs: u64,
    pub all_day: bool,
}

/// Fetch today's events from the Google Calendar API (OAuth).
/// `singleEvents=true` expands recurring events server-side, so this is
/// more accurate than the ICS/RRULE fallback.
pub async fn today_events_google(access_token: &str) -> Result<Vec<CalEvent>, String> {
    let today = Local::now().date_naive();
    let t0 = today
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .ok_or("time error")?;
    let t1 = today
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .ok_or("time error")?;

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| e.to_string())?;
    let res = http
        .get("https://www.googleapis.com/calendar/v3/calendars/primary/events")
        .bearer_auth(access_token)
        .query(&[
            ("timeMin", t0.to_rfc3339()),
            ("timeMax", t1.to_rfc3339()),
            ("singleEvents", "true".into()),
            ("orderBy", "startTime".into()),
            ("maxResults", "100".into()),
        ])
        .send()
        .await
        .map_err(|e| format!("โหลด calendar ไม่ได้: {e}"))?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or(serde_json::Value::Null);
    if !status.is_success() {
        return Err(format!("Google Calendar ตอบกลับ {status}"));
    }

    let mut out = vec![];
    for item in body["items"].as_array().unwrap_or(&vec![]) {
        if item["status"].as_str() == Some("cancelled") {
            continue;
        }
        let summary = item["summary"].as_str().unwrap_or("(ไม่มีชื่อ)").to_string();
        // all-day events use start.date, timed events use start.dateTime
        if let Some(_d) = item["start"]["date"].as_str() {
            out.push(CalEvent {
                summary,
                start: t0.to_rfc3339(),
                end: t1.to_rfc3339(),
                duration_secs: 0,
                all_day: true,
            });
            continue;
        }
        let (Some(s), Some(e)) = (
            item["start"]["dateTime"].as_str(),
            item["end"]["dateTime"].as_str(),
        ) else {
            continue;
        };
        let (Ok(sdt), Ok(edt)) = (
            DateTime::parse_from_rfc3339(s),
            DateTime::parse_from_rfc3339(e),
        ) else {
            continue;
        };
        let s_local = sdt.with_timezone(&Local);
        let e_local = edt.with_timezone(&Local);
        out.push(CalEvent {
            summary,
            start: s_local.to_rfc3339(),
            end: e_local.to_rfc3339(),
            duration_secs: (e_local - s_local).num_seconds().max(0) as u64,
            all_day: false,
        });
    }
    Ok(out)
}

/// Fetch the ICS feed and return today's events (local date).
pub async fn today_events(ics_url: &str) -> Result<Vec<CalEvent>, String> {
    if ics_url.trim().is_empty() {
        return Err("ยังไม่ได้ตั้งค่า Calendar ICS URL".into());
    }
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| e.to_string())?;
    let raw = http
        .get(ics_url.trim())
        .send()
        .await
        .map_err(|e| format!("โหลด calendar ไม่ได้: {e}"))?
        .error_for_status()
        .map_err(|e| format!("โหลด calendar ไม่ได้: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let today = Local::now().date_naive();
    Ok(parse_today(&raw, today))
}

fn parse_today(raw: &str, today: NaiveDate) -> Vec<CalEvent> {
    let lines = unfold(raw);
    let mut events = vec![];
    let mut cur: Option<Vec<(String, String, String)>> = None; // (name, params, value)

    for line in &lines {
        if line == "BEGIN:VEVENT" {
            cur = Some(vec![]);
        } else if line == "END:VEVENT" {
            if let Some(props) = cur.take() {
                if let Some(ev) = build_event(&props, today) {
                    events.push(ev);
                }
            }
        } else if let Some(props) = cur.as_mut() {
            if let Some((head, value)) = line.split_once(':') {
                let (name, params) = match head.split_once(';') {
                    Some((n, p)) => (n.to_string(), p.to_string()),
                    None => (head.to_string(), String::new()),
                };
                props.push((name.to_uppercase(), params, value.to_string()));
            }
        }
    }

    events.sort_by(|a, b| a.start.cmp(&b.start));
    events
}

/// Unfold RFC5545 folded lines (continuations start with space/tab).
fn unfold(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in raw.lines() {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.push_str(&line[1..]);
        } else {
            out.push(line.trim_end_matches('\r').to_string());
        }
    }
    out
}

fn prop<'a>(props: &'a [(String, String, String)], name: &str) -> Option<&'a (String, String, String)> {
    props.iter().find(|(n, _, _)| n == name)
}

fn build_event(props: &[(String, String, String)], today: NaiveDate) -> Option<CalEvent> {
    let summary = prop(props, "SUMMARY")
        .map(|(_, _, v)| unescape(v))
        .unwrap_or_default();
    if summary.is_empty() {
        return None;
    }
    if prop(props, "STATUS").map(|(_, _, v)| v.as_str()) == Some("CANCELLED") {
        return None;
    }

    let (_, dtstart_params, dtstart_val) = prop(props, "DTSTART")?;
    let start = parse_dt(dtstart_params, dtstart_val)?;

    let end = match prop(props, "DTEND") {
        Some((_, p, v)) => parse_dt(p, v)?,
        None => match &start {
            DtValue::AllDay(d) => DtValue::AllDay(*d + Duration::days(1)),
            DtValue::Timed(t) => DtValue::Timed(*t + Duration::hours(1)),
        },
    };

    // Which occurrence date applies today?
    let occurs = match prop(props, "RRULE") {
        Some((_, _, rrule)) => occurs_today(&start, rrule, props, today),
        None => start.local_date() == today,
    };
    if !occurs {
        return None;
    }

    match (&start, &end) {
        (DtValue::AllDay(_), _) => Some(CalEvent {
            summary,
            start: today.and_hms_opt(0, 0, 0)?.and_local_timezone(Local).single()?.to_rfc3339(),
            end: today.and_hms_opt(23, 59, 0)?.and_local_timezone(Local).single()?.to_rfc3339(),
            duration_secs: 0,
            all_day: true,
        }),
        (DtValue::Timed(s), DtValue::Timed(e)) => {
            let dur = (*e - *s).num_seconds().max(0) as u64;
            // for recurring events, shift the original start time onto today
            let s_local = s.with_timezone(&Local);
            let start_today = today
                .and_time(s_local.time())
                .and_local_timezone(Local)
                .single()?;
            let end_today = start_today + Duration::seconds(dur as i64);
            Some(CalEvent {
                summary,
                start: start_today.to_rfc3339(),
                end: end_today.to_rfc3339(),
                duration_secs: dur,
                all_day: false,
            })
        }
        _ => None,
    }
}

enum DtValue {
    AllDay(NaiveDate),
    Timed(DateTime<Utc>),
}

impl DtValue {
    fn local_date(&self) -> NaiveDate {
        match self {
            DtValue::AllDay(d) => *d,
            DtValue::Timed(t) => t.with_timezone(&Local).date_naive(),
        }
    }
    fn start_date_for_rrule(&self) -> NaiveDate {
        self.local_date()
    }
}

fn parse_dt(params: &str, value: &str) -> Option<DtValue> {
    let value = value.trim();
    if params.contains("VALUE=DATE") || (value.len() == 8 && !value.contains('T')) {
        let d = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some(DtValue::AllDay(d));
    }
    if let Some(stripped) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some(DtValue::Timed(Utc.from_utc_datetime(&naive)));
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    // TZID=Asia/Bangkok etc.
    let tzid = params
        .split(';')
        .find_map(|p| p.strip_prefix("TZID="))
        .unwrap_or("");
    if let Ok(tz) = Tz::from_str(tzid) {
        let dt = tz.from_local_datetime(&naive).single()?;
        return Some(DtValue::Timed(dt.with_timezone(&Utc)));
    }
    // floating time — assume local
    let dt = Local.from_local_datetime(&naive).single()?;
    Some(DtValue::Timed(dt.with_timezone(&Utc)))
}

/// Minimal RRULE evaluation: DAILY / WEEKLY (with BYDAY, INTERVAL, UNTIL, COUNT approx) + EXDATE.
fn occurs_today(
    start: &DtValue,
    rrule: &str,
    props: &[(String, String, String)],
    today: NaiveDate,
) -> bool {
    let start_date = start.start_date_for_rrule();
    if today < start_date {
        return false;
    }

    let mut freq = "";
    let mut interval: i64 = 1;
    let mut until: Option<NaiveDate> = None;
    let mut byday: Vec<Weekday> = vec![];
    for part in rrule.split(';') {
        let (k, v) = match part.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match k {
            "FREQ" => freq = v,
            "INTERVAL" => interval = v.parse().unwrap_or(1),
            "UNTIL" => {
                let d = &v[..8.min(v.len())];
                until = NaiveDate::parse_from_str(d, "%Y%m%d").ok();
            }
            "BYDAY" => {
                byday = v
                    .split(',')
                    .filter_map(|d| match d.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-' || c == '+') {
                        "MO" => Some(Weekday::Mon),
                        "TU" => Some(Weekday::Tue),
                        "WE" => Some(Weekday::Wed),
                        "TH" => Some(Weekday::Thu),
                        "FR" => Some(Weekday::Fri),
                        "SA" => Some(Weekday::Sat),
                        "SU" => Some(Weekday::Sun),
                        _ => None,
                    })
                    .collect();
            }
            _ => {}
        }
    }

    if let Some(u) = until {
        if today > u {
            return false;
        }
    }

    // EXDATE check
    for (name, _, v) in props {
        if name == "EXDATE" {
            for ex in v.split(',') {
                let d = &ex[..8.min(ex.len())];
                if NaiveDate::parse_from_str(d, "%Y%m%d").ok() == Some(today) {
                    return false;
                }
            }
        }
    }

    match freq {
        "DAILY" => (today - start_date).num_days() % interval == 0,
        "WEEKLY" => {
            let dow_ok = if byday.is_empty() {
                today.weekday() == start_date.weekday()
            } else {
                byday.contains(&today.weekday())
            };
            if !dow_ok {
                return false;
            }
            // weeks since start (Monday-aligned)
            let align = |d: NaiveDate| d - Duration::days(d.weekday().num_days_from_monday() as i64);
            let weeks = (align(today) - align(start_date)).num_days() / 7;
            weeks % interval == 0
        }
        "MONTHLY" => today.day() == start_date.day(),
        "YEARLY" => today.day() == start_date.day() && today.month() == start_date.month(),
        _ => false,
    }
}

fn unescape(v: &str) -> String {
    v.replace("\\n", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}
