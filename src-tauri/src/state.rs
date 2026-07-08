use chrono::{DateTime, Local, NaiveDate};
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

use crate::jira::IssueLite;
use crate::oauth::Connections;
use crate::settings::Settings;
use crate::winstate::WindowState;

#[derive(Debug, Clone)]
pub struct ActiveTimer {
    pub issue_key: String,
    pub started_at: DateTime<Local>,
    pub started_instant: Instant,
}

pub struct AppState {
    pub config_dir: PathBuf,
    pub settings: RwLock<Settings>,
    pub connections: RwLock<Connections>,
    /// window position + pin state — std Mutex because it is touched from
    /// synchronous window-event callbacks (no await held)
    pub window_state: std::sync::Mutex<WindowState>,
    pub issues: RwLock<Vec<IssueLite>>,
    pub sprint_name: RwLock<Option<String>>,
    pub issues_fetched_at: RwLock<Option<Instant>>,
    /// Jira metadata that rarely changes — cleared when the Jira identity
    /// might change (login/logout/settings save) so it re-resolves lazily
    pub sprint_field_id: RwLock<Option<String>>,
    pub account_id: RwLock<Option<String>>,
    pub today_secs: RwLock<Option<u64>>,
    pub timer: Mutex<Option<ActiveTimer>>,
    pub last_periodic_notify: Mutex<Option<Instant>>,
    pub eod_notified_on: Mutex<Option<NaiveDate>>,
}

impl AppState {
    pub fn new(
        config_dir: PathBuf,
        settings: Settings,
        connections: Connections,
        window_state: WindowState,
    ) -> Self {
        Self {
            config_dir,
            settings: RwLock::new(settings),
            connections: RwLock::new(connections),
            window_state: std::sync::Mutex::new(window_state),
            issues: RwLock::new(vec![]),
            sprint_name: RwLock::new(None),
            issues_fetched_at: RwLock::new(None),
            sprint_field_id: RwLock::new(None),
            account_id: RwLock::new(None),
            today_secs: RwLock::new(None),
            timer: Mutex::new(None),
            last_periodic_notify: Mutex::new(None),
            eod_notified_on: Mutex::new(None),
        }
    }
}
