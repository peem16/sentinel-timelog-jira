use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tauri::{AppHandle, Manager, State};

use crate::calendar::{self, CalEvent};
use crate::github::{self, ReviewedPR};
use crate::gitscan::{self, BranchInfo};
use crate::jira::{IssueLite, JiraClient, ProjectLite, SprintLite, WorkType};
use crate::logged;
use crate::oauth;
use crate::settings::{self, Settings};
use crate::state::{ActiveTimer, AppState};

const ISSUE_CACHE_SECS: u64 = 300;

/// Sprint custom-field id, resolved once per Jira identity and cached.
pub async fn cached_sprint_field(app: &AppHandle, client: &JiraClient) -> Result<String, String> {
    let state = app.state::<AppState>();
    if let Some(f) = state.sprint_field_id.read().await.clone() {
        return Ok(f);
    }
    let f = client.sprint_field_id().await?;
    *state.sprint_field_id.write().await = Some(f.clone());
    Ok(f)
}

/// The user's Jira accountId, resolved once per Jira identity and cached.
pub async fn cached_account_id(app: &AppHandle, client: &JiraClient) -> Result<String, String> {
    let state = app.state::<AppState>();
    if let Some(id) = state.account_id.read().await.clone() {
        return Ok(id);
    }
    let id = client.my_account_id().await?;
    *state.account_id.write().await = Some(id.clone());
    Ok(id)
}

/// Drop cached Jira metadata — call whenever the Jira identity may change.
async fn invalidate_jira_caches(state: &AppState) {
    *state.issues_fetched_at.write().await = None;
    *state.sprint_field_id.write().await = None;
    *state.account_id.write().await = None;
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    new_settings: Settings,
) -> Result<(), String> {
    let (old_hotkey, old_create_hotkey) = {
        let s = state.settings.read().await;
        (s.hotkey.clone(), s.create_hotkey.clone())
    };
    settings::save(&state.config_dir, &new_settings)?;
    *state.settings.write().await = new_settings.clone();
    // invalidate caches that depend on settings (project / manual creds may change)
    invalidate_jira_caches(&state).await;

    let hotkey_result = if old_hotkey != new_settings.hotkey
        || old_create_hotkey != new_settings.create_hotkey
    {
        crate::register_hotkey(&app, &new_settings.hotkey, &new_settings.create_hotkey)
    } else {
        Ok(())
    };
    crate::refresh_total_and_tray(app.clone()).await;
    // settings are saved either way — but tell the user if the hotkey failed
    hotkey_result
}

#[derive(Debug, Serialize)]
pub struct SprintIssues {
    pub sprint_name: Option<String>,
    pub issues: Vec<IssueLite>,
}

#[tauri::command]
pub async fn get_sprint_issues(
    app: AppHandle,
    force: Option<bool>,
) -> Result<SprintIssues, String> {
    let state = app.state::<AppState>();
    let force = force.unwrap_or(false);
    if !force {
        let fetched = *state.issues_fetched_at.read().await;
        if let Some(t) = fetched {
            if t.elapsed().as_secs() < ISSUE_CACHE_SECS {
                return Ok(SprintIssues {
                    sprint_name: state.sprint_name.read().await.clone(),
                    issues: state.issues.read().await.clone(),
                });
            }
        }
    }
    let s = state.settings.read().await.clone();
    let client = oauth::jira_client(&app).await?;
    let sprint_field = cached_sprint_field(&app, &client).await?;
    let (sprint_name, issues) = client.sprint_issues(&s.jira_project_key, &sprint_field).await?;
    *state.issues.write().await = issues.clone();
    *state.sprint_name.write().await = sprint_name.clone();
    *state.issues_fetched_at.write().await = Some(Instant::now());
    Ok(SprintIssues { sprint_name, issues })
}

#[derive(Debug, Serialize)]
pub struct TaskFormMeta {
    pub work_types: Vec<WorkType>,
    pub sprints: Vec<SprintLite>,
    pub default_sprint_id: Option<u64>,
}

/// Work types + selectable sprints for the Create-Task form.
#[tauri::command]
pub async fn get_task_form_meta(app: AppHandle) -> Result<TaskFormMeta, String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().await.clone();
    let client = oauth::jira_client(&app).await?;
    let sprint_field = cached_sprint_field(&app, &client).await?;
    let work_types = client.issue_types(&s.jira_project_key).await?;
    let sprints = client.list_sprints(&s.jira_project_key, &sprint_field).await?;
    let default_sprint_id = sprints
        .iter()
        .find(|sp| sp.state == "active")
        .or_else(|| sprints.first())
        .map(|sp| sp.id);
    Ok(TaskFormMeta {
        work_types,
        sprints,
        default_sprint_id,
    })
}

/// Create a Jira issue and return its new key. Refreshes the sprint-issue cache
/// so the new task shows up in the picker.
#[tauri::command]
pub async fn create_issue(
    app: AppHandle,
    issue_type_id: String,
    issue_type_name: String,
    summary: String,
    description: String,
    sprint_id: Option<u64>,
) -> Result<String, String> {
    if summary.trim().is_empty() {
        return Err("กรอกหัวข้อก่อน".into());
    }
    let state = app.state::<AppState>();
    let s = state.settings.read().await.clone();
    let client = oauth::jira_client(&app).await?;
    let sprint_field = cached_sprint_field(&app, &client).await?;
    let key = client
        .create_issue(
            &s.jira_project_key,
            &issue_type_id,
            &issue_type_name,
            &summary,
            &description,
            sprint_id,
            &sprint_field,
        )
        .await?;
    // force the sprint-issue list to refetch next time so the new task appears
    *state.issues_fetched_at.write().await = None;
    Ok(key)
}

#[tauri::command]
pub async fn get_today_total(app: AppHandle, force: Option<bool>) -> Result<u64, String> {
    let state = app.state::<AppState>();
    if !force.unwrap_or(false) {
        if let Some(cached) = *state.today_secs.read().await {
            return Ok(cached);
        }
    }
    let s = state.settings.read().await.clone();
    let client = oauth::jira_client(&app).await?;
    let account_id = cached_account_id(&app, &client).await?;
    let total = client.today_logged_seconds(&account_id).await?;
    *state.today_secs.write().await = Some(total);
    crate::update_tray(&app, Some(total), s.work_hours_per_day);
    Ok(total)
}

#[tauri::command]
pub async fn get_stack(state: State<'_, AppState>) -> Result<crate::streak::StackStatus, String> {
    let s = state.settings.read().await.clone();
    if !s.stack_enabled {
        return Ok(crate::streak::StackStatus {
            current: 0,
            best: 0,
            qualified_today: false,
            enabled: false,
        });
    }
    Ok(crate::streak::status(&state.config_dir, true))
}

#[tauri::command]
pub async fn log_work(
    app: AppHandle,
    issue_key: String,
    seconds: u64,
    comment: String,
    started: Option<String>,
) -> Result<u64, String> {
    if seconds == 0 {
        return Err("เวลาต้องมากกว่า 0".into());
    }
    let state = app.state::<AppState>();
    let s = state.settings.read().await.clone();
    let client = oauth::jira_client(&app).await?;
    client.log_work(&issue_key, seconds, &comment, started).await?;

    // optimistic local update, then refresh from Jira in the background
    let new_total = {
        let mut cached = state.today_secs.write().await;
        let t = cached.unwrap_or(0) + seconds;
        *cached = Some(t);
        t
    };
    crate::update_tray(&app, Some(new_total), s.work_hours_per_day);
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::refresh_total_and_tray(app2).await;
    });
    Ok(new_total)
}

#[tauri::command]
pub async fn get_branches(state: State<'_, AppState>) -> Result<Vec<BranchInfo>, String> {
    let s = state.settings.read().await.clone();
    let roots = s.workspace_roots.clone();
    let key = s.jira_project_key.clone();
    // fs scan — run off the async thread
    tauri::async_runtime::spawn_blocking(move || Ok(gitscan::scan(&roots, &key))).await
        .map_err(|e| e.to_string())?
}

#[derive(Debug, Serialize)]
pub struct AutoSuggestion {
    pub event: CalEvent,
    pub issue_key: Option<String>,
    pub issue_summary: Option<String>,
}

#[tauri::command]
pub async fn get_auto_suggestions(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<AutoSuggestion>, String> {
    let s = state.settings.read().await.clone();
    // prefer the Google OAuth connection; fall back to the ICS url
    let has_google = state.connections.read().await.google.is_some();
    let has_ics = !s.ics_url.trim().is_empty();
    let events = if has_google {
        let google = match oauth::fresh_google(&app).await {
            Ok(conn) => calendar::today_events_google(&conn.tokens.access_token).await,
            Err(e) => Err(e),
        };
        match google {
            Ok(ev) => ev,
            // Google broke mid-day — the ICS url still works as a fallback
            Err(e) if has_ics => calendar::today_events(&s.ics_url).await.map_err(|_| e)?,
            Err(e) => return Err(e),
        }
    } else if has_ics {
        calendar::today_events(&s.ics_url).await?
    } else {
        // no calendar source configured — not an error, the auto tab may be
        // used for GitHub PRs only
        vec![]
    };
    let issues = get_sprint_issues(app.clone(), None).await?.issues;

    let out = events
        .into_iter()
        .map(|ev| {
            let title = ev.summary.to_lowercase();
            let matched = s
                .auto_rules
                .iter()
                .find(|r| {
                    !r.calendar_keyword.trim().is_empty()
                        && title.contains(&r.calendar_keyword.trim().to_lowercase())
                })
                .and_then(|rule| {
                    let prefix = rule.jira_prefix.trim().to_lowercase();
                    if prefix.is_empty() {
                        return None;
                    }
                    issues
                        .iter()
                        .find(|i| i.summary.to_lowercase().starts_with(&prefix))
                        .or_else(|| {
                            issues.iter().find(|i| i.summary.to_lowercase().contains(&prefix))
                        })
                });
            AutoSuggestion {
                event: ev,
                issue_key: matched.map(|i| i.key.clone()),
                issue_summary: matched.map(|i| i.summary.clone()),
            }
        })
        .collect();
    Ok(out)
}

#[derive(Debug, Deserialize)]
pub struct AutoEntry {
    pub issue_key: String,
    pub seconds: u64,
    pub comment: String,
    pub started: Option<String>,
    /// opaque dedupe key for the source item (PR url / calendar event) — marked
    /// as "logged today" once the worklog succeeds so it isn't logged twice
    pub key: Option<String>,
}

#[tauri::command]
pub async fn confirm_auto(app: AppHandle, entries: Vec<AutoEntry>) -> Result<u64, String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().await.clone();
    let client = oauth::jira_client(&app).await?;
    let mut logged: u64 = 0;
    let mut errors: Vec<String> = vec![];
    let mut logged_keys: Vec<String> = vec![];
    for e in &entries {
        if e.seconds == 0 {
            continue;
        }
        match client
            .log_work(&e.issue_key, e.seconds, &e.comment, e.started.clone())
            .await
        {
            Ok(_) => {
                logged += e.seconds;
                if let Some(k) = &e.key {
                    logged_keys.push(k.clone());
                }
            }
            Err(err) => errors.push(format!("{}: {}", e.issue_key, err)),
        }
    }
    // remember what succeeded (even on partial failure) so re-fetch won't re-log
    if !logged_keys.is_empty() {
        let _ = logged::add(&state.config_dir, &logged_keys);
    }
    let new_total = {
        let mut cached = state.today_secs.write().await;
        let t = cached.unwrap_or(0) + logged;
        *cached = Some(t);
        t
    };
    crate::update_tray(&app, Some(new_total), s.work_hours_per_day);
    if !errors.is_empty() {
        return Err(errors.join(" | "));
    }
    Ok(new_total)
}

/// Keys of auto-tab items already logged today — the frontend badges these as
/// "ลงแล้ว" and blocks re-logging.
#[tauri::command]
pub async fn get_logged_keys(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(logged::today_keys(&state.config_dir))
}

/// PRs the user reviewed/commented on today (GitHub), each mapped to a Jira
/// key parsed from the PR title. The frontend folds these into the Auto tab.
#[tauri::command]
pub async fn get_reviewed_prs(app: AppHandle) -> Result<Vec<ReviewedPR>, String> {
    let state = app.state::<AppState>();
    let s = state.settings.read().await.clone();
    let conn = oauth::fresh_github(&app).await?;
    let client = github::GitHubClient::bearer(conn.tokens.access_token.clone())?;
    let mut prs = client.reviewed_today(&s.github_org, &conn.login).await?;

    // map each PR to a Jira key from its title, enriching with the sprint
    // summary when the key belongs to the current sprint
    let issues = get_sprint_issues(app.clone(), None)
        .await
        .map(|r| r.issues)
        .unwrap_or_default();
    for pr in &mut prs {
        pr.issue_key = gitscan::extract_issue_key(&pr.title, &s.jira_project_key);
        pr.issue_summary = pr.issue_key.as_ref().and_then(|k| {
            issues
                .iter()
                .find(|i| i.key.eq_ignore_ascii_case(k))
                .map(|i| i.summary.clone())
        });
    }
    Ok(prs)
}

#[derive(Debug, Serialize)]
pub struct TimerStatus {
    pub running: bool,
    pub issue_key: Option<String>,
    pub elapsed_secs: u64,
    pub started_at: Option<String>,
}

/// Seconds of the lunch break that fall inside `[start, now]`, so the timer can
/// exclude it. Handles the rare multi-day session by summing each day's window.
fn lunch_overlap_secs(start: DateTime<Local>, now: DateTime<Local>, s: &Settings) -> u64 {
    if !s.lunch_enabled || now <= start {
        return 0;
    }
    let (Some(ls), Some(le)) = (
        crate::parse_hhmm(&s.lunch_start),
        crate::parse_hhmm(&s.lunch_end),
    ) else {
        return 0;
    };
    if le <= ls {
        return 0;
    }
    let mut total: i64 = 0;
    let mut day = start.date_naive();
    let last = now.date_naive();
    loop {
        if let (Some(lo), Some(hi)) = (
            Local.from_local_datetime(&day.and_time(ls)).single(),
            Local.from_local_datetime(&day.and_time(le)).single(),
        ) {
            let ov_lo = start.max(lo);
            let ov_hi = now.min(hi);
            if ov_hi > ov_lo {
                total += (ov_hi - ov_lo).num_seconds();
            }
        }
        if day >= last {
            break;
        }
        match day.succ_opt() {
            Some(d) => day = d,
            None => break,
        }
    }
    total.max(0) as u64
}

#[tauri::command]
pub async fn timer_start(state: State<'_, AppState>, issue_key: String) -> Result<TimerStatus, String> {
    let mut timer = state.timer.lock().await;
    if timer.is_some() {
        return Err("มี timer ที่กำลังจับเวลาอยู่แล้ว".into());
    }
    let t = ActiveTimer {
        issue_key: issue_key.clone(),
        started_at: Local::now(),
        started_instant: Instant::now(),
    };
    *timer = Some(t.clone());
    Ok(TimerStatus {
        running: true,
        issue_key: Some(issue_key),
        elapsed_secs: 0,
        started_at: Some(t.started_at.to_rfc3339()),
    })
}

#[tauri::command]
pub async fn timer_stop(state: State<'_, AppState>) -> Result<TimerStatus, String> {
    let s = state.settings.read().await.clone();
    let mut timer = state.timer.lock().await;
    match timer.take() {
        Some(t) => {
            let raw = t.started_instant.elapsed().as_secs();
            let lunch = lunch_overlap_secs(t.started_at, Local::now(), &s);
            Ok(TimerStatus {
                running: false,
                issue_key: Some(t.issue_key.clone()),
                elapsed_secs: raw.saturating_sub(lunch),
                started_at: Some(t.started_at.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string()),
            })
        }
        None => Err("ไม่มี timer ที่กำลังทำงาน".into()),
    }
}

#[tauri::command]
pub async fn timer_status(state: State<'_, AppState>) -> Result<TimerStatus, String> {
    let s = state.settings.read().await.clone();
    let timer = state.timer.lock().await;
    Ok(match &*timer {
        Some(t) => TimerStatus {
            running: true,
            issue_key: Some(t.issue_key.clone()),
            elapsed_secs: t
                .started_instant
                .elapsed()
                .as_secs()
                .saturating_sub(lunch_overlap_secs(t.started_at, Local::now(), &s)),
            started_at: Some(t.started_at.to_rfc3339()),
        },
        None => TimerStatus {
            running: false,
            issue_key: None,
            elapsed_secs: 0,
            started_at: None,
        },
    })
}

#[tauri::command]
pub async fn hide_window(app: AppHandle) -> Result<(), String> {
    crate::hide_main_window(&app);
    Ok(())
}

/// Frontend signals a drag started (mousedown on a drag region) / ended
/// (mouseup). Guards the panel against the auto-hide-on-blur during a drag.
#[tauri::command]
pub fn begin_drag() {
    crate::set_dragging(true);
}

#[tauri::command]
pub fn end_drag() {
    crate::set_dragging(false);
}

#[tauri::command]
pub async fn toggle_pin(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let pinned = {
        let mut ws = state.window_state.lock().map_err(|e| e.to_string())?;
        ws.pinned = !ws.pinned;
        ws.pinned
    };
    crate::persist_window_state(&app);
    Ok(pinned)
}

#[tauri::command]
pub async fn get_pinned(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let pinned = state.window_state.lock().map_err(|e| e.to_string())?.pinned;
    Ok(pinned)
}

/* ================= OAuth connections ================= */

#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    pub atlassian_connected: bool,
    pub atlassian_site: Option<String>,
    pub google_connected: bool,
    pub google_email: Option<String>,
    pub github_connected: bool,
    pub github_login: Option<String>,
    /// credentials compiled into this build — UI hides the manual fields
    pub atlassian_embedded: bool,
    pub google_embedded: bool,
    pub github_embedded: bool,
}

async fn status_of(state: &AppState) -> ConnectionStatus {
    let conns = state.connections.read().await;
    ConnectionStatus {
        atlassian_connected: conns.atlassian.is_some(),
        atlassian_site: conns.atlassian.as_ref().map(|a| {
            if a.site_name.is_empty() { a.site_url.clone() } else { a.site_name.clone() }
        }),
        google_connected: conns.google.is_some(),
        google_email: conns.google.as_ref().map(|g| g.email.clone()),
        github_connected: conns.github.is_some(),
        github_login: conns.github.as_ref().map(|g| g.login.clone()),
        atlassian_embedded: oauth::atlassian_embedded(),
        google_embedded: oauth::google_embedded(),
        github_embedded: oauth::github_embedded(),
    }
}

#[tauri::command]
pub async fn connection_status(state: State<'_, AppState>) -> Result<ConnectionStatus, String> {
    Ok(status_of(&state).await)
}

#[tauri::command]
pub async fn connect_provider(app: AppHandle, provider: String) -> Result<ConnectionStatus, String> {
    match provider.as_str() {
        "atlassian" => {
            oauth::connect_atlassian(&app).await?;
            // fresh data with the new identity
            let state = app.state::<AppState>();
            invalidate_jira_caches(&state).await;
            *state.today_secs.write().await = None;
            crate::refresh_total_and_tray(app.clone()).await;
        }
        "google" => {
            oauth::connect_google(&app).await?;
        }
        "github" => {
            oauth::connect_github(&app).await?;
        }
        other => return Err(format!("ไม่รู้จัก provider: {other}")),
    }
    Ok(status_of(&app.state::<AppState>()).await)
}

#[tauri::command]
pub async fn disconnect_provider(app: AppHandle, provider: String) -> Result<ConnectionStatus, String> {
    let state = app.state::<AppState>();
    {
        let mut conns = state.connections.write().await;
        match provider.as_str() {
            "atlassian" => conns.atlassian = None,
            "google" => conns.google = None,
            "github" => conns.github = None,
            other => return Err(format!("ไม่รู้จัก provider: {other}")),
        }
    }
    if provider == "atlassian" {
        invalidate_jira_caches(&state).await;
    }
    let conns = state.connections.read().await.clone();
    oauth::save(&conns)?;
    Ok(status_of(&state).await)
}

#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectLite>, String> {
    let client = oauth::jira_client(&app).await?;
    client.list_projects().await
}

/* ================= autostart / updater ================= */

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|e| format!("เปิด autostart ไม่ได้: {e}"))
    } else {
        autolaunch.disable().map_err(|e| format!("ปิด autostart ไม่ได้: {e}"))
    }
}

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// Check the configured update endpoint. Returns None when already current.
/// Errors with a friendly message when the updater isn't configured (no
/// endpoint/pubkey in tauri.conf.json — see README).
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app
        .updater()
        .map_err(|e| format!("updater ยังไม่ได้ตั้งค่า (ดู README หัวข้อ Auto-update): {e}"))?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo {
            version: u.version.clone(),
            notes: u.body.clone(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("ตรวจอัปเดตไม่สำเร็จ: {e}")),
    }
}

/// Download + install the available update, then restart into the new build.
/// (On Windows the installer closes the app itself.)
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Err("ไม่มีอัปเดตให้ติดตั้ง".into());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("ดาวน์โหลด/ติดตั้งอัปเดตไม่สำเร็จ: {e}"))?;
    crate::persist_window_state(&app);
    app.restart();
}

/// Probe each Jira endpoint the app relies on and report per-step results, so a
/// 401/empty problem can be pinpointed (which scope / which call / which site).
#[tauri::command]
pub async fn diagnose_jira(app: AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let project = state.settings.read().await.jira_project_key.clone();
    let site = state
        .connections
        .read()
        .await
        .atlassian
        .as_ref()
        .map(|a| format!("{} ({})", a.site_name, a.site_url));

    let mut out = String::new();
    out.push_str(&format!("site: {}\n", site.as_deref().unwrap_or("— ยังไม่ได้ login —")));
    out.push_str(&format!("project key: {}\n", if project.is_empty() { "(ว่าง!)" } else { &project }));

    let client = match oauth::jira_client(&app).await {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("\n✗ สร้าง client ไม่ได้: {e}"));
            return Ok(out);
        }
    };

    match client.my_account_id().await {
        Ok(id) => out.push_str(&format!("\n✓ /myself OK (read:jira-user) — {}…", &id[..id.len().min(8)])),
        Err(e) => out.push_str(&format!("\n✗ /myself (ต้องมี read:jira-user):\n{e}")),
    }

    match client.list_projects().await {
        Ok(ps) => {
            let has = ps.iter().any(|p| p.key.eq_ignore_ascii_case(&project));
            out.push_str(&format!(
                "\n\n✓ /project/search OK — เห็น {} project{}",
                ps.len(),
                if has { format!(", มี {project} ✓") } else { format!(", แต่ไม่เห็น {project} ✗") }
            ));
        }
        Err(e) => out.push_str(&format!("\n\n✗ /project/search:\n{e}")),
    }

    match client.sprint_field_id().await {
        Ok(field) => {
            out.push_str(&format!("\n\n✓ Sprint field: {field}"));
            match client.sprint_issues(&project, &field).await {
                Ok((name, is)) => out.push_str(&format!(
                    "\n✓ sprint OK — {} task ใน sprint: {}",
                    is.len(),
                    name.as_deref().unwrap_or("(ไม่พบ sprint)")
                )),
                Err(e) => out.push_str(&format!("\n✗ sprint (JQL):\n{e}")),
            }
        }
        Err(e) => out.push_str(&format!("\n\n✗ หา Sprint field ไม่เจอ:\n{e}")),
    }

    Ok(out)
}
