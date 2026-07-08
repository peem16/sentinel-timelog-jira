use serde::Serialize;
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
}

/// Scan workspace roots for git repos (root itself, or 1 level deep) and
/// report the branch each repo is currently on, most recently active first.
pub fn scan(roots: &[String], project_key: &str) -> Vec<BranchInfo> {
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

    let mut out: Vec<BranchInfo> = repos
        .into_iter()
        .filter_map(|repo| {
            let branch = read_branch(&repo)?;
            let issue_key = extract_issue_key(&branch, project_key);
            let idle_secs = last_activity(&repo)
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .map(|d| d.as_secs())
                .unwrap_or(u64::MAX);
            Some(BranchInfo {
                repo: repo
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: repo.to_string_lossy().to_string(),
                branch,
                issue_key,
                idle_secs,
            })
        })
        .collect();

    out.sort_by_key(|b| b.idle_secs);
    out
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
