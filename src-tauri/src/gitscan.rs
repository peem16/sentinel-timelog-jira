use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub repo: String,
    pub path: String,
    pub branch: String,
    pub issue_key: Option<String>,
    /// seconds since last git activity (HEAD/index mtime)
    pub idle_secs: u64,
    /// IDE that currently has this folder open ("Cursor" / "VS Code" / …),
    /// or None if the repo was found only by scanning workspace_roots.
    pub ide: Option<String>,
    /// true when this is the IDE's currently-focused (last-active) window.
    pub active: bool,
}

/// Report the branch of each folder an IDE (Cursor / VS Code / Windsurf) that is
/// **currently running** has open — the IDE's focused window first, then its
/// other windows. `workspace_roots` still seed candidates (so a root that the
/// IDE has open gets a proper repo name), but a repo only surfaces when a live
/// IDE actually has it open: no IDE running → no chips. storage.json keeps a
/// `lastActiveWindow` even after the IDE closes, so the running-process check is
/// what keeps stale, closed-editor branches from showing.
pub fn scan(roots: &[String], project_key: &str) -> Vec<BranchInfo> {
    // normalized path -> (ide label, is this the focused window?)
    let ide_folders = ide_open_folders();
    let mut ide_map: HashMap<String, (String, bool)> = HashMap::new();
    for (ide, path, active) in &ide_folders {
        ide_map
            .entry(norm(path))
            .and_modify(|e| e.1 |= *active)
            .or_insert_with(|| (ide.clone(), *active));
    }

    // candidate repos: workspace roots (root itself, or 1 level deep) …
    let mut repos: Vec<PathBuf> = vec![];
    for root in roots {
        let root = PathBuf::from(root);
        if root.join(".git").exists() {
            repos.push(root.clone());
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() && p.join(".git").exists() {
                    repos.push(p);
                }
            }
        }
    }
    // … plus every folder an IDE has open that is itself a git repo
    for (_, path, _) in &ide_folders {
        let p = PathBuf::from(path);
        if p.join(".git").exists() {
            repos.push(p);
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<BranchInfo> = repos
        .into_iter()
        .filter(|repo| seen.insert(norm(&repo.to_string_lossy())))
        .filter_map(|repo| {
            let branch = read_branch(&repo)?;
            let issue_key = extract_issue_key(&branch, project_key);
            let idle_secs = last_activity(&repo)
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX);
            let (ide, active) = match ide_map.get(&norm(&repo.to_string_lossy())) {
                Some((label, active)) => (Some(label.clone()), *active),
                None => (None, false),
            };
            Some(BranchInfo {
                repo: repo
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: repo.to_string_lossy().to_string(),
                branch,
                issue_key,
                idle_secs,
                ide,
                active,
            })
        })
        .collect();

    // only surface repos a running IDE actually has open — drop root-only repos
    out.retain(|b| b.ide.is_some());
    // focused IDE window first, then any IDE window, then most recently active
    out.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.ide.is_some().cmp(&a.ide.is_some()))
            .then(a.idle_secs.cmp(&b.idle_secs))
    });
    out
}

/// Case-insensitive, slash-normalized path key so folders reported by an IDE
/// (`d:/Work/foo`) match repos scanned from `workspace_roots` (`D:\Work\foo`).
fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Folders currently open in Cursor / VS Code / Windsurf, read from each IDE's
/// `globalStorage/storage.json` (`windowsState`). No IDE process is touched —
/// just the on-disk state file, which the IDE rewrites when windows open, close,
/// or refocus. Returns (ide label, folder path, is_focused_window).
fn ide_open_folders() -> Vec<(String, String, bool)> {
    let running = running_ides();
    let mut out = vec![];
    for (label, storage) in ide_storage_files() {
        if !running.contains(&label) {
            continue; // IDE not open right now — its stored window state is stale
        }
        let Ok(raw) = std::fs::read_to_string(&storage) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let ws = &json["windowsState"];
        if let Some(p) = window_folder(&ws["lastActiveWindow"]) {
            out.push((label.clone(), p, true));
        }
        if let Some(arr) = ws["openedWindows"].as_array() {
            for w in arr {
                if let Some(p) = window_folder(w) {
                    out.push((label.clone(), p, false));
                }
            }
        }
    }
    out
}

/// The folder a window has open — a single-folder window (`folder`) or the
/// directory containing a multi-root workspace file (`workspace.configPath`).
fn window_folder(w: &serde_json::Value) -> Option<String> {
    if let Some(uri) = w["folder"].as_str() {
        return uri_to_path(uri);
    }
    if let Some(uri) = w["workspace"]["configPath"].as_str() {
        let p = uri_to_path(uri)?;
        return Path::new(&p).parent().map(|d| d.to_string_lossy().to_string());
    }
    None
}

/// (label, storage.json path) for each VS Code-family IDE we know about.
fn ide_storage_files() -> Vec<(String, PathBuf)> {
    let Some(base) = ide_config_base() else {
        return vec![];
    };
    [
        ("Cursor", "Cursor"),
        ("Code", "VS Code"),
        ("Windsurf", "Windsurf"),
        ("Code - Insiders", "VS Code Insiders"),
        ("VSCodium", "VSCodium"),
    ]
    .iter()
    .map(|(dir, label)| {
        let p = base
            .join(dir)
            .join("User")
            .join("globalStorage")
            .join("storage.json");
        (label.to_string(), p)
    })
    .collect()
}

/// Roaming config base where VS Code-family IDEs keep their per-user state.
fn ide_config_base() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    }
}

/// Decode a `file://` URI from storage.json into a filesystem path.
/// e.g. `file:///d%3A/Work/foo` -> `d:/Work/foo` on Windows.
fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = urlencoding::decode(rest).ok()?.into_owned();
    #[cfg(target_os = "windows")]
    {
        // drop the leading slash before the drive letter: "/d:/…" -> "d:/…"
        Some(decoded.strip_prefix('/').unwrap_or(&decoded).to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Some(decoded)
    }
}

/// Labels of the VS Code-family IDEs whose process is running right now.
/// Matched against `ide_storage_files()` so a stored window only counts when its
/// editor is actually open.
fn running_ides() -> std::collections::HashSet<String> {
    let names = process_names();
    // exact (lowercased, ".exe"-stripped) process name -> UI label
    let map = [
        ("cursor", "Cursor"),
        ("code", "VS Code"),
        ("windsurf", "Windsurf"),
        ("code - insiders", "VS Code Insiders"),
        ("vscodium", "VSCodium"),
        ("codium", "VSCodium"),
    ];
    let mut set = std::collections::HashSet::new();
    for name in &names {
        for (exe, label) in map {
            if name == exe {
                set.insert(label.to_string());
            }
        }
    }
    set
}

/// Running process image names, lowercased and without a trailing `.exe`.
fn process_names() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        // CSV, no header — first field of each row is "ImageName"
        run_hidden("tasklist", &["/FO", "CSV", "/NH"])
            .map(|out| {
                out.lines()
                    .filter_map(|l| l.split("\",\"").next())
                    .map(|s| s.trim_matches('"').trim_end_matches(".exe").to_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "windows"))]
    {
        run_hidden("ps", &["-Ao", "comm="])
            .map(|out| {
                out.lines()
                    .filter_map(|l| {
                        Path::new(l.trim())
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Run a short command and capture stdout, without popping a console window on
/// Windows (this app has no console of its own).
fn run_hidden(cmd: &str, args: &[&str]) -> Option<String> {
    let mut c = std::process::Command::new(cmd);
    c.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = c.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Extract a Jira-style issue key from arbitrary text (a branch name, a PR
/// title like "[MDT-1728] External BFF", …). Prefers the configured project
/// prefix; falls back to a generic `KEY-123` match. Returns the key uppercased.
pub fn extract_issue_key(text: &str, project_key: &str) -> Option<String> {
    let generic = || regex::Regex::new(r"([A-Za-z][A-Za-z0-9]+-\d+)").unwrap();
    let re = if project_key.trim().is_empty() {
        generic()
    } else {
        regex::Regex::new(&format!(r"(?i)({}-\d+)", regex::escape(project_key)))
            .unwrap_or_else(|_| generic())
    };
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_uppercase())
}

fn read_branch(repo: &Path) -> Option<String> {
    let head = std::fs::read_to_string(repo.join(".git").join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref: refs/heads/") {
        Some(rest.to_string())
    } else {
        // detached HEAD — short sha
        Some(head.chars().take(8).collect())
    }
}

fn last_activity(repo: &Path) -> Option<SystemTime> {
    let git = repo.join(".git");
    let mut newest: Option<SystemTime> = None;
    for f in ["index", "HEAD", "FETCH_HEAD", "COMMIT_EDITMSG"] {
        if let Ok(meta) = std::fs::metadata(git.join(f)) {
            if let Ok(m) = meta.modified() {
                newest = Some(match newest {
                    Some(cur) if cur > m => cur,
                    _ => m,
                });
            }
        }
    }
    newest
}
