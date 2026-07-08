//! Tiny per-day store of "already logged" auto-tab items, so a PR/calendar
//! entry that was logged earlier today shows a "ลงแล้ว" badge and can't be
//! logged again — even after re-fetching or restarting the app.
//!
//! Keys are opaque strings minted by the frontend (`pr:<url>` for a GitHub PR,
//! `cal:<summary>|<start>` for a calendar event). The store resets when the
//! local date rolls over.

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct LoggedStore {
    /// local date (YYYY-MM-DD) these keys belong to
    date: String,
    keys: Vec<String>,
}

fn store_path(config_dir: &Path) -> PathBuf {
    config_dir.join("logged.json")
}

fn load_raw(config_dir: &Path) -> LoggedStore {
    std::fs::read_to_string(store_path(config_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// Keys logged today (empty if the stored day isn't today).
pub fn today_keys(config_dir: &Path) -> Vec<String> {
    let s = load_raw(config_dir);
    if s.date == today() {
        s.keys
    } else {
        vec![]
    }
}

/// Record keys as logged today (deduped; resets the store on a new day).
pub fn add(config_dir: &Path, keys: &[String]) -> Result<(), String> {
    let today = today();
    let mut s = load_raw(config_dir);
    if s.date != today {
        s.date = today;
        s.keys.clear();
    }
    for k in keys {
        if !k.is_empty() && !s.keys.contains(k) {
            s.keys.push(k.clone());
        }
    }
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    std::fs::write(store_path(config_dir), raw).map_err(|e| e.to_string())
}
