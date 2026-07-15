use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default AI CLI command. NOTE: do NOT use `--bare` here — it skips loading
/// the CLI's stored credentials and every call fails with "Not logged in".
/// `--strict-mcp-config` gives the startup speedup (no MCP servers) safely.
/// `--allowedTools "Read Grep Glob Bash(git:*)"` lets the `mdt-task-writer`
/// skill read the code (fill Reference `path:line`) and run git for permalink
/// SHAs without prompting in headless mode — read/git only, no Edit/Write/general Bash.
pub const DEFAULT_AI_COMMAND: &str =
    "claude -p --strict-mcp-config --allowedTools \"Read Grep Glob Bash(git:*)\" --output-format json";

/// Skill that Claude Code must load for every draft. `build_prompt` prepends
/// `/<skill>` to the prompt so the CLI resolves it as a slash command — a hard
/// invocation, unlike hoping the model picks the skill up from prose. Leave the
/// setting blank to draft without a skill.
pub const DEFAULT_AI_SKILL: &str = "mdt-task-writer";

/// Older shipped defaults, migrated forward in `load()` so existing users pick
/// up the current tool set without re-typing the command themselves.
const LEGACY_AI_COMMANDS: &[&str] = &[
    "claude -p --bare --output-format json",
    "claude -p --strict-mcp-config --output-format json",
    "claude -p --strict-mcp-config --allowedTools \"Read Grep Glob\" --output-format json",
];

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
    /// Show the daily streak "stack" badge on the summary card
    pub stack_enabled: bool,
    /// Hours that must be logged on a working day to count a stack (+1 streak)
    pub stack_threshold_hours: f64,
    /// Remind every N minutes when hours are not complete (0 = off)
    pub remind_every_minutes: u64,
    /// Remind N minutes before end of day if incomplete (0 = off)
    pub remind_before_end_minutes: u64,
    /// "HH:MM" local time
    pub end_of_day: String,
    /// Exclude the lunch window from the running timer
    pub lunch_enabled: bool,
    /// "HH:MM" local time — start of lunch break
    pub lunch_start: String,
    /// "HH:MM" local time — end of lunch break
    pub lunch_end: String,
    /// Global hotkey to toggle the panel
    pub hotkey: String,
    /// Global hotkey to open the Create-Task form (empty = off)
    pub create_hotkey: String,
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
    /// AI-assisted issue writing: toggle state, persisted from the Create Task form
    pub ai_enabled: bool,
    /// Shell command that reads the prompt from stdin and prints the answer to stdout.
    /// The prompt delegates the issue structure to the `ai_skill` skill.
    pub ai_command: String,
    /// Skill name forced on every draft (prepended as `/<name>` to the prompt).
    /// Blank = no skill. See `DEFAULT_AI_SKILL`. Field-level default so an older
    /// `settings.json` missing the key upgrades to the skill, while a user who
    /// deliberately saved it blank keeps it blank.
    #[serde(default = "default_ai_skill")]
    pub ai_skill: String,
}

fn default_ai_skill() -> String {
    DEFAULT_AI_SKILL.into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            jira_base_url: String::new(),
            jira_email: String::new(),
            jira_api_token: String::new(),
            jira_project_key: "MDT".into(),
            work_hours_per_day: 8.0,
            stack_enabled: true,
            stack_threshold_hours: 6.0,
            remind_every_minutes: 120,
            remind_before_end_minutes: 30,
            end_of_day: "18:00".into(),
            lunch_enabled: true,
            lunch_start: "12:00".into(),
            lunch_end: "13:00".into(),
            hotkey: "Ctrl+Alt+L".into(),
            create_hotkey: "Ctrl+Alt+K".into(),
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
            ai_enabled: false,
            ai_command: DEFAULT_AI_COMMAND.into(),
            ai_skill: DEFAULT_AI_SKILL.into(),
        }
    }
}

pub fn settings_path(config_dir: &PathBuf) -> PathBuf {
    config_dir.join("settings.json")
}

pub fn load(config_dir: &PathBuf) -> Settings {
    let path = settings_path(config_dir);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let mut s: Settings = serde_json::from_str(&raw).unwrap_or_default();
            // migrate any older shipped default to the current one: the `--bare`
            // builds fail with "Not logged in", and the pre-skill / read-only
            // defaults lack the tools the skill now needs. A user-customized
            // command is left untouched.
            if LEGACY_AI_COMMANDS.contains(&s.ai_command.as_str()) {
                s.ai_command = DEFAULT_AI_COMMAND.into();
            }
            s
        }
        Err(_) => Settings::default(),
    }
}

pub fn save(config_dir: &PathBuf, settings: &Settings) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(config_dir), raw).map_err(|e| e.to_string())
}
