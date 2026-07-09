//! Daily "stack" streak: a working day (Mon–Fri) where the logged total
//! reaches the threshold (default 6h) counts as +1. Consecutive qualifying
//! working days build a streak; missing a working day resets it. Weekends are
//! ignored — not logging Sat/Sun never breaks the streak.
//!
//! Persisted in `streak.json` next to the other per-user stores (mirrors
//! `logged.rs` / `winstate.rs`). The frontend renders a pixel-medal badge whose
//! tier is derived from `current`.

use chrono::{Datelike, Local, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct StackStore {
    /// streak count as of `last_qualified`
    current: u32,
    /// best streak ever reached
    best: u32,
    /// local date (YYYY-MM-DD) of the most recent qualifying working day
    last_qualified: String,
}

/// Display-ready status handed to the frontend.
#[derive(Serialize, Clone)]
pub struct StackStatus {
    /// streak to show (0 = broken and today not yet qualified)
    pub current: u32,
    pub best: u32,
    pub qualified_today: bool,
    pub enabled: bool,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("streak.json")
}

fn load(dir: &Path) -> StackStore {
    std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

fn save(dir: &Path, s: &StackStore) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(path(dir), raw).map_err(|e| e.to_string())
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn is_working_day(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// The working day immediately before `d` (skips Sat/Sun).
fn prev_working_day(d: NaiveDate) -> NaiveDate {
    let mut p = d.pred_opt().unwrap_or(d);
    while !is_working_day(p) {
        p = p.pred_opt().unwrap_or(p);
    }
    p
}

/// Pure display rule: the streak to show given the stored count, the last
/// qualifying date string, and today. Returns (current_to_show, qualified_today).
/// The streak stays "alive" (shows `stored_current`) while `last_qualified` is
/// today or the previous working day; once a working day is missed it reads 0.
fn display_current(stored_current: u32, last_qualified: &str, today: NaiveDate) -> (u32, bool) {
    let today_str = today.to_string();
    let qualified_today = last_qualified == today_str;
    let alive = qualified_today || last_qualified == prev_working_day(today).to_string();
    (if alive { stored_current } else { 0 }, qualified_today)
}

/// Pure qualify rule: the new stored streak when today (a working day) first
/// crosses the threshold. `None` = already counted today (no change).
fn qualified_new_current(stored_current: u32, last_qualified: &str, today: NaiveDate) -> Option<u32> {
    let today_str = today.to_string();
    if last_qualified == today_str {
        None
    } else if last_qualified == prev_working_day(today).to_string() {
        Some(stored_current + 1)
    } else {
        Some(1)
    }
}

/// Compute the display status without writing.
pub fn status(dir: &Path, enabled: bool) -> StackStatus {
    let s = load(dir);
    let (current, qualified_today) = display_current(s.current, &s.last_qualified, today());
    StackStatus {
        current,
        best: s.best,
        qualified_today,
        enabled,
    }
}

/// Evaluate today's total against the threshold, updating the store when a
/// working day first crosses it. Returns the fresh display status.
pub fn evaluate(dir: &Path, threshold_secs: u64, today_total_secs: u64) -> StackStatus {
    let t = today();
    if is_working_day(t) && today_total_secs >= threshold_secs {
        let mut s = load(dir);
        if let Some(new_current) = qualified_new_current(s.current, &s.last_qualified, t) {
            s.current = new_current;
            s.last_qualified = t.to_string();
            s.best = s.best.max(s.current);
            let _ = save(dir, &s);
        }
    }
    status(dir, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn working_days_and_prev() {
        // 2026-07-09 is a Thursday, 07-10 Fri, 07-11 Sat, 07-12 Sun, 07-13 Mon
        assert!(is_working_day(d("2026-07-10"))); // Fri
        assert!(!is_working_day(d("2026-07-11"))); // Sat
        assert!(!is_working_day(d("2026-07-12"))); // Sun
        assert!(is_working_day(d("2026-07-13"))); // Mon
        // Monday's previous working day skips the weekend back to Friday
        assert_eq!(prev_working_day(d("2026-07-13")), d("2026-07-10"));
        // Thursday's previous working day is Wednesday
        assert_eq!(prev_working_day(d("2026-07-09")), d("2026-07-08"));
    }

    #[test]
    fn qualify_continues_across_weekend() {
        // last qualified Fri, today Mon -> streak continues (5 -> 6)
        assert_eq!(qualified_new_current(5, "2026-07-10", d("2026-07-13")), Some(6));
    }

    #[test]
    fn qualify_continues_consecutive_weekdays() {
        // last qualified Wed, today Thu -> 3 -> 4
        assert_eq!(qualified_new_current(3, "2026-07-08", d("2026-07-09")), Some(4));
    }

    #[test]
    fn qualify_resets_after_missed_working_day() {
        // last qualified Wed, today Fri (missed Thu) -> reset to 1
        assert_eq!(qualified_new_current(9, "2026-07-08", d("2026-07-10")), Some(1));
    }

    #[test]
    fn qualify_noop_when_already_counted_today() {
        assert_eq!(qualified_new_current(4, "2026-07-09", d("2026-07-09")), None);
    }

    #[test]
    fn display_alive_before_logging_today() {
        // qualified yesterday (Wed), today Thu not yet logged -> still shows 4, not qualified
        assert_eq!(display_current(4, "2026-07-08", d("2026-07-09")), (4, false));
    }

    #[test]
    fn display_alive_over_weekend() {
        // qualified Fri, today Sat -> streak alive, shows 5
        assert_eq!(display_current(5, "2026-07-10", d("2026-07-11")), (5, false));
    }

    #[test]
    fn display_broken_after_missed_day() {
        // qualified Wed, today Fri (missed Thu) -> shows 0
        assert_eq!(display_current(9, "2026-07-08", d("2026-07-10")), (0, false));
    }

    #[test]
    fn display_counts_today_when_qualified() {
        assert_eq!(display_current(6, "2026-07-13", d("2026-07-13")), (6, true));
    }
}
