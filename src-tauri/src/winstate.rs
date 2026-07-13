use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persisted position + size + pin state of the widget window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    /// true once the window has been placed at least once (so we restore
    /// the saved position instead of re-snapping to the tray)
    pub has_pos: bool,
    /// pinned = stays on screen (does not auto-hide when it loses focus)
    pub pinned: bool,
    /// physical inner size (== outer size, the window is undecorated)
    pub width: u32,
    pub height: u32,
    /// true once the user has resized at least once
    pub has_size: bool,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("window.json")
}

pub fn load(dir: &Path) -> WindowState {
    std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, ws: &WindowState) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(ws).map_err(|e| e.to_string())?;
    std::fs::write(path(dir), raw).map_err(|e| e.to_string())
}
