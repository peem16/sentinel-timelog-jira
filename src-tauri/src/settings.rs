use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoRule {
    /// Keyword to look for in a calendar event title, e.g. "Mandrake Grooming"
    pub calendar_keyword: String,
    /// Prefix of the Jira issue summary to match against, e.g. "Grooming"
    pub jira_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub jira_base_url: String,
    pub jira_email: String,
    pub jira_api_token: String,
    pub jira_project_key: String,
    /// Target working hours per day
    pub work_hours_per_day: f64,
    /// Remind every N minutes when hours are not complete (0 = off)
    pub remind_every_minutes: u64,
    /// Remind N minutes before end of day if incomplete (0 = off)
    pub remind_before_end_minutes: u64,
    /// "HH:MM" local time
    pub end_of_day: String,
    /// Global hotkey to toggle the panel
    pub hotkey: String,
    /// Directories that contain git repos (scanned 1 level deep)
    pub workspace_roots: Vec<String>,
    /// Google Calendar secret ICS url
    pub ics_url: String,
    pub auto_rules: Vec<AutoRule>,
    /// How often to refresh the tray total from Jira (minutes)
    pub refresh_interval_minutes: u64,
    /// OAuth app credentials (one-time registration, see README)
    pub atlassian_client_id: String,
    pub atlassian_client_secret: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    /// GitHub org to search for reviewed PRs (e.g. "wisesight")
    pub github_org: String,
    pub github_client_id: String,
    pub github_client_secret: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            jira_base_url: String::new(),
            jira_email: String::new(),
            jira_api_token: String::new(),
            jira_project_key: "MDT".into(),
            work_hours_per_day: 8.0,
            remind_every_minutes: 120,
            remind_before_end_minutes: 30,
            end_of_day: "18:00".into(),
            hotkey: "Ctrl+Alt+L".into(),
            workspace_roots: vec![],
            ics_url: String::new(),
            auto_rules: vec![],
            refresh_interval_minutes: 15,
            atlassian_client_id: String::new(),
            atlassian_client_secret: String::new(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            github_org: String::new(),
            github_client_id: String::new(),
            github_client_secret: String::new(),
        }
    }
}

pub fn settings_path(config_dir: &PathBuf) -> PathBuf {
    config_dir.join("settings.json")
}

pub fn load(config_dir: &PathBuf) -> Settings {
    let path = settings_path(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(config_dir: &PathBuf, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(config_dir), raw).map_err(|e| e.to_string())
}
