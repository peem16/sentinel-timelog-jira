mod calendar;
mod commands;
mod github;
mod gitscan;
mod jira;
mod logged;
mod oauth;
mod settings;
mod state;
mod streak;
mod tray;
mod winstate;

use chrono::{Local, NaiveTime, Timelike};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_positioner::{Position, WindowExt};

use crate::state::AppState;

const TRAY_ID: &str = "main-tray";

/// Last time the panel was hidden — used to swallow the tray-click that
/// caused a blur-hide, so clicking the tray while open closes the panel.
static LAST_HIDE: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

fn mark_hidden() {
    *LAST_HIDE.lock().unwrap() = Some(Instant::now());
}

fn just_hidden() -> bool {
    LAST_HIDE
        .lock()
        .unwrap()
        .map(|t| t.elapsed() < Duration::from_millis(300))
        .unwrap_or(false)
}

/// True while the user is dragging the widget — set by the frontend on
/// mousedown over a drag region. Prevents the blur that Windows fires at the
/// start of a window move-loop from auto-hiding the panel mid-drag.
static DRAGGING: AtomicBool = AtomicBool::new(false);

pub fn set_dragging(v: bool) {
    DRAGGING.store(v, Ordering::SeqCst);
}

pub fn is_dragging() -> bool {
    DRAGGING.load(Ordering::SeqCst)
}

pub fn toggle_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide_and_save(app, &win);
    } else if !just_hidden() {
        show_window(app, &win);
    }
}

fn show_window(app: &AppHandle, win: &WebviewWindow) {
    // restore the last dragged position; otherwise dock next to the tray
    let saved = app.state::<AppState>().window_state.lock().unwrap().clone();
    // size first: the TrayCenter fallback below positions based on current size
    if saved.has_size && saved.width > 0 && saved.height > 0 {
        let _ = win.set_size(tauri::PhysicalSize::new(saved.width, saved.height));
    }
    if saved.has_pos {
        let _ = win.set_position(tauri::PhysicalPosition::new(saved.x, saved.y));
    } else if win.move_window(Position::TrayCenter).is_err() {
        #[cfg(target_os = "macos")]
        let _ = win.move_window(Position::TopRight);
        #[cfg(not(target_os = "macos"))]
        let _ = win.move_window(Position::BottomRight);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = win.emit("panel-shown", ());
}

/// Capture the window's current position + size into app state.
fn capture_position(app: &AppHandle, win: &WebviewWindow) {
    let state = app.state::<AppState>();
    let mut ws = state.window_state.lock().unwrap();
    if let Ok(pos) = win.outer_position() {
        ws.x = pos.x;
        ws.y = pos.y;
        ws.has_pos = true;
    }
    if let Ok(size) = win.inner_size() {
        if size.width > 0 && size.height > 0 {
            ws.width = size.width;
            ws.height = size.height;
            ws.has_size = true;
        }
    }
}

/// Persist the in-memory window state (position + pin) to disk.
pub fn persist_window_state(app: &AppHandle) {
    let state = app.state::<AppState>();
    let ws = state.window_state.lock().unwrap().clone();
    let _ = winstate::save(&state.config_dir, &ws);
}

fn is_pinned(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let pinned = state.window_state.lock().unwrap().pinned;
    pinned
}

/// Hide the panel, remembering where it was so it reopens in the same spot.
fn hide_and_save(app: &AppHandle, win: &WebviewWindow) {
    capture_position(app, win);
    persist_window_state(app);
    mark_hidden();
    let _ = win.hide();
}

/// Hide the main window from a command context.
pub fn hide_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        hide_and_save(app, &win);
    }
}

/// Update the tray to show e.g. 6.2 / 8. `done_secs = None` renders "-".
pub fn update_tray(app: &AppHandle, done_secs: Option<u64>, target_hours: f64) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let done = match done_secs {
        Some(s) => tray::fmt_hours(s),
        None => "-".to_string(),
    };
    let total = format!("/{}", tray::fmt_target(target_hours));
    let tooltip = format!("TimeLog — วันนี้ {done}{total} ชม.");

    #[cfg(target_os = "macos")]
    {
        let _ = tray.set_title(Some(format!("{done}{total}")));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let icon = tray::render_fraction_icon(&done, &total);
        let _ = tray.set_icon(Some(icon));
    }
    let _ = tray.set_tooltip(Some(tooltip));
}

/// Fetch today's logged total from Jira, cache it, update tray + notify UI.
pub async fn refresh_total_and_tray(app: AppHandle) {
    let state = app.state::<AppState>();
    let s = state.settings.read().await.clone();
    let Ok(client) = oauth::jira_client(&app).await else {
        update_tray(&app, None, s.work_hours_per_day);
        return;
    };
    let account_id = match commands::cached_account_id(&app, &client).await {
        Ok(id) => id,
        Err(_) => {
            let cached = *state.today_secs.read().await;
            update_tray(&app, cached, s.work_hours_per_day);
            return;
        }
    };
    match client.today_logged_seconds(&account_id).await {
        Ok(total) => {
            *state.today_secs.write().await = Some(total);
            update_tray(&app, Some(total), s.work_hours_per_day);
            let _ = app.emit("total-updated", total);
            if s.stack_enabled {
                let threshold = (s.stack_threshold_hours * 3600.0) as u64;
                let st = streak::evaluate(&state.config_dir, threshold, total);
                let _ = app.emit("stack-updated", st);
            }
        }
        Err(_) => {
            let cached = *state.today_secs.read().await;
            update_tray(&app, cached, s.work_hours_per_day);
        }
    }
}

pub fn register_hotkey(app: &AppHandle, hotkey: &str, create_hotkey: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if !hotkey.trim().is_empty() {
        gs.on_shortcut(hotkey, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                toggle_window(app);
            }
        })
        .map_err(|e| format!("ลงทะเบียน hotkey '{hotkey}' ไม่ได้ (อาจชนกับโปรแกรมอื่น): {e}"))?;
    }
    if !create_hotkey.trim().is_empty() {
        gs.on_shortcut(create_hotkey, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                if let Some(win) = app.get_webview_window("main") {
                    show_window(app, &win);
                    let _ = win.emit("open-create", ());
                }
            }
        })
        .map_err(|e| {
            format!("ลงทะเบียน hotkey สร้าง task '{create_hotkey}' ไม่ได้ (อาจชนกับโปรแกรมอื่น): {e}")
        })?;
    }
    Ok(())
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

pub(crate) fn parse_hhmm(v: &str) -> Option<NaiveTime> {
    let (h, m) = v.split_once(':')?;
    NaiveTime::from_hms_opt(h.trim().parse().ok()?, m.trim().parse().ok()?, 0)
}

async fn scheduler(app: AppHandle) {
    let mut last_refresh: Option<Instant> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let state = app.state::<AppState>();
        let s = state.settings.read().await.clone();

        // cheap insurance: flush window position for a pinned widget that
        // may never fire a hide event
        persist_window_state(&app);

        // periodic refresh of the tray total
        let refresh_due = last_refresh
            .map(|t| t.elapsed().as_secs() >= s.refresh_interval_minutes.max(1) * 60)
            .unwrap_or(true);
        if refresh_due {
            last_refresh = Some(Instant::now());
            refresh_total_and_tray(app.clone()).await;
        }

        // no Jira configured (fresh install) → nothing to remind about
        let jira_configured = state.connections.read().await.atlassian.is_some()
            || (!s.jira_base_url.is_empty()
                && !s.jira_email.is_empty()
                && !s.jira_api_token.is_empty());
        if !jira_configured {
            continue;
        }

        // past end of day → stop working: once per day, notify + stop the timer
        // (keeping elapsed for logging) + pop the window up pinned so it stays.
        // Runs before the "target met" early-out below so it fires regardless.
        {
            let now = Local::now();
            let end = parse_hhmm(&s.end_of_day)
                .unwrap_or_else(|| NaiveTime::from_hms_opt(18, 0, 0).unwrap());
            if now.time() >= end {
                let mut done = state.work_ended_on.lock().await;
                if *done != Some(now.date_naive()) {
                    *done = Some(now.date_naive());
                    drop(done);
                    notify(&app, "หมดเวลาทำงานแล้ว", "หยุดจับเวลาและลงเวลาที่เหลือได้เลย");
                    // pin so the auto-hide-on-blur won't hide the panel
                    {
                        let mut ws = state.window_state.lock().unwrap();
                        ws.pinned = true;
                    }
                    persist_window_state(&app);
                    if let Some(win) = app.get_webview_window("main") {
                        show_window(&app, &win);
                    }
                    // frontend stops the timer + prefills the log form
                    let _ = app.emit("work-ended", ());
                }
            }
        }

        let target_secs = (s.work_hours_per_day * 3600.0) as u64;
        let total = state.today_secs.read().await.unwrap_or(0);
        if total >= target_secs {
            continue;
        }

        let now = Local::now();
        let end = parse_hhmm(&s.end_of_day).unwrap_or_else(|| NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        let now_t = now.time();
        let missing_h = tray::fmt_hours(target_secs - total);

        // remind every N minutes during working hours (09:00 → end of day)
        if s.remind_every_minutes > 0
            && now_t >= NaiveTime::from_hms_opt(9, 0, 0).unwrap()
            && now_t < end
        {
            let mut last = state.last_periodic_notify.lock().await;
            let due = last
                .map(|t| t.elapsed().as_secs() >= s.remind_every_minutes * 60)
                .unwrap_or(true);
            if due {
                *last = Some(Instant::now());
                notify(
                    &app,
                    "อย่าลืมลงเวลา",
                    &format!(
                        "วันนี้ลงไป {}/{} ชม. ยังขาดอีก {missing_h} ชม.",
                        tray::fmt_hours(total),
                        tray::fmt_target(s.work_hours_per_day)
                    ),
                );
            }
        }

        // remind before end of day (once per day)
        if s.remind_before_end_minutes > 0 {
            let warn_at_secs = end.num_seconds_from_midnight() as i64
                - (s.remind_before_end_minutes as i64) * 60;
            let now_secs = now_t.num_seconds_from_midnight() as i64;
            if now_secs >= warn_at_secs && now_secs < end.num_seconds_from_midnight() as i64 {
                let mut done_on = state.eod_notified_on.lock().await;
                if *done_on != Some(now.date_naive()) {
                    *done_on = Some(now.date_naive());
                    notify(
                        &app,
                        "ใกล้หมดวันแล้ว",
                        &format!(
                            "ลงเวลาไป {}/{} ชม. — ยังขาด {missing_h} ชม. ก่อน {}",
                            tray::fmt_hours(total),
                            tray::fmt_target(s.work_hours_per_day),
                            s.end_of_day
                        ),
                    );
                }
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_sprint_issues,
            commands::get_sprint_issues_all,
            commands::get_task_form_meta,
            commands::create_issue,
            commands::get_today_total,
            commands::get_stack,
            commands::log_work,
            commands::get_branches,
            commands::get_auto_suggestions,
            commands::get_reviewed_prs,
            commands::confirm_auto,
            commands::get_logged_keys,
            commands::timer_start,
            commands::timer_stop,
            commands::timer_status,
            commands::hide_window,
            commands::begin_drag,
            commands::end_drag,
            commands::open_url,
            commands::toggle_pin,
            commands::get_pinned,
            commands::connection_status,
            commands::connect_provider,
            commands::disconnect_provider,
            commands::list_projects,
            commands::diagnose_jira,
            commands::app_version,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app
                .path()
                .app_config_dir()
                .expect("no app config dir");
            let loaded = settings::load(&config_dir);
            let connections = oauth::load(&config_dir);
            let window_state = winstate::load(&config_dir);
            let hotkey = loaded.hotkey.clone();
            let create_hotkey = loaded.create_hotkey.clone();
            let target_hours = loaded.work_hours_per_day;
            app.manage(AppState::new(config_dir, loaded, connections, window_state));

            // ----- window setup -----
            // No native vibrancy/acrylic on either OS — the glass tone is baked into
            // the CSS gradient (styles.css .widget) so both platforms look the same.
            //
            // macOS note: we used to apply_vibrancy(HudWindow). That material puts the
            // window into an NSPanel HUD style that reserves a titlebar strip, which
            // shoved the webview content up ~12px and clipped the header / dropdowns /
            // overlays at the top edge. Windows never had vibrancy (apply_acrylic
            // paints an opaque rectangle that leaks through the transparent
            // rounded-corner notches as a gray square), so dropping it on macOS too
            // gives one consistent look and fixes the clipping.
            if let Some(win) = app.get_webview_window("main") {
                // track drags + auto-hide on blur (unless pinned)
                let win2 = win.clone();
                let app_handle = app.handle().clone();
                win.on_window_event(move |event| match event {
                    // remember the position live while the user drags the widget
                    tauri::WindowEvent::Moved(pos) => {
                        let state = app_handle.state::<AppState>();
                        let mut ws = state.window_state.lock().unwrap();
                        ws.x = pos.x;
                        ws.y = pos.y;
                        ws.has_pos = true;
                    }
                    // remember the size live while the user resizes
                    // (0×0 fires on minimize — don't persist that)
                    tauri::WindowEvent::Resized(size) => {
                        if size.width > 0 && size.height > 0 {
                            let state = app_handle.state::<AppState>();
                            let mut ws = state.window_state.lock().unwrap();
                            ws.width = size.width;
                            ws.height = size.height;
                            ws.has_size = true;
                        }
                    }
                    // drag finished (or user clicked back in) — clear the drag guard
                    tauri::WindowEvent::Focused(true) => {
                        set_dragging(false);
                    }
                    // pinned widget stays put; otherwise behave like a tray popup.
                    // Defer the hide slightly: Windows fires this blur at the very
                    // start of a drag, before the frontend's begin_drag IPC lands.
                    tauri::WindowEvent::Focused(false) => {
                        if is_pinned(&app_handle) {
                            return;
                        }
                        let app2 = app_handle.clone();
                        let win3 = win2.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(250)).await;
                            // still a real click-away? (not dragging, not refocused, not pinned)
                            if is_dragging() || is_pinned(&app2) {
                                return;
                            }
                            if win3.is_focused().unwrap_or(false) {
                                return;
                            }
                            hide_and_save(&app2, &win3);
                        });
                    }
                    _ => {}
                });
            }

            // ----- tray -----
            let open_item = MenuItem::with_id(app, "open", "เปิด TimeLog", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "รีเฟรชชั่วโมงจาก Jira", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "ออกจากโปรแกรม", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &refresh_item, &sep, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::with_id(TRAY_ID)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => toggle_window(app),
                    "refresh" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            refresh_total_and_tray(app).await;
                        });
                    }
                    "quit" => {
                        persist_window_state(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray_icon, event| {
                    tauri_plugin_positioner::on_tray_event(tray_icon.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(tray_icon.app_handle());
                    }
                });

            #[cfg(not(target_os = "macos"))]
            {
                tray_builder = tray_builder.icon(tray::render_fraction_icon(
                    "-",
                    &format!("/{}", tray::fmt_target(target_hours)),
                ));
            }
            #[cfg(target_os = "macos")]
            {
                if let Some(icon) = app.default_window_icon() {
                    tray_builder = tray_builder.icon(icon.clone()).icon_as_template(true);
                }
                tray_builder = tray_builder.title(format!("-/{}", tray::fmt_target(target_hours)));
            }
            tray_builder.build(app)?;

            // ----- hotkey -----
            if let Err(e) = register_hotkey(app.handle(), &hotkey, &create_hotkey) {
                eprintln!("{e}");
            }

            // ----- initial fetch + background scheduler -----
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                refresh_total_and_tray(handle.clone()).await;
                scheduler(handle).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running timelog");
}
