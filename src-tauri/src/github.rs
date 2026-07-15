use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

const API: &str = "https://api.github.com";

#[derive(Debug, Clone, Serialize)]
pub struct ReviewedPR {
    /// "owner/name"
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// login of the PR author (not the reviewer)
    pub author: String,
    /// local RFC3339 time of the user's latest review/comment today (None if the
    /// verification calls failed and the exact time is unknown)
    pub reviewed_at: Option<String>,
    /// Jira key parsed from the title, filled in by the command layer
    pub issue_key: Option<String>,
    pub issue_summary: Option<String>,
}

pub struct GitHubClient {
    http: reqwest::Client,
    token: String,
}

impl GitHubClient {
    pub fn bearer(token: String) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { http, token })
    }

    async fn get(&self, url: &str, query: &[(&str, String)]) -> Result<Value, String> {
        let res = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            // GitHub rejects requests without a User-Agent
            .header("User-Agent", "timelog")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(query)
            .send()
            .await
            .map_err(|e| format!("GitHub request ล้มเหลว: {e}"))?;
        let status = res.status();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(api_err(status, url, &body));
        }
        Ok(body)
    }

    /// PRs in `org` that the user (login) reviewed or commented on **today**
    /// (local date). Comment/Approve/Request-changes all count.
    pub async fn reviewed_today(&self, org: &str, login: &str) -> Result<Vec<ReviewedPR>, String> {
        let org = org.trim();
        if org.is_empty() {
            return Err("ยังไม่ได้ตั้งค่า GitHub Org ในหน้า Settings".into());
        }
        let today = Local::now().format("%Y-%m-%d").to_string();

        // stage 1 — candidate PRs. Search's date qualifier filters on the PR's
        // `updated` field (not the review time), so this only narrows the set;
        // stage 2 verifies the review/comment actually happened today. The two
        // searches are independent, so run them concurrently.
        let search_url = format!("{API}/search/issues");
        let query = |qual: String| {
            vec![
                ("q", format!("is:pr org:{org} {qual} updated:>={today}")),
                ("per_page", "50".to_string()),
            ]
        };
        let q_rev = query(format!("reviewed-by:{login}"));
        let q_com = query(format!("commenter:{login}"));
        let (rev, com) = tokio::join!(
            self.get(&search_url, &q_rev),
            self.get(&search_url, &q_com),
        );

        let mut candidates: HashMap<String, Value> = HashMap::new();
        for res in [rev?, com?] {
            for item in res["items"].as_array().unwrap_or(&vec![]) {
                if let Some(url) = item["html_url"].as_str() {
                    candidates.entry(url.to_string()).or_insert_with(|| item.clone());
                }
            }
        }

        // stage 2 — verify each candidate concurrently. Every PR needs two GET
        // calls (reviews + comments); doing all PRs in parallel turns what was
        // 2×N serial roundtrips into a single concurrent wave.
        let today = today.as_str(); // Copy, so each async block can capture it
        let verified = futures::future::join_all(candidates.values().map(|item| async move {
            let html_url = item["html_url"].as_str().unwrap_or_default().to_string();
            let number = item["number"].as_u64().unwrap_or(0);
            let title = item["title"].as_str().unwrap_or_default().to_string();
            let author = item["user"]["login"].as_str().unwrap_or_default().to_string();
            let (owner, repo) =
                parse_owner_repo(item["repository_url"].as_str().unwrap_or_default())?;

            // Ok(None) = verified nothing today (skip); Err = the verification
            // calls failed, so keep the candidate with unknown time.
            let reviewed_at = match self.last_action_today(&owner, &repo, number, login, &today).await
            {
                Ok(Some(dt)) => Some(dt),
                Ok(None) => return None,
                Err(_) => None,
            };

            Some(ReviewedPR {
                repo: format!("{owner}/{repo}"),
                number,
                title,
                url: html_url,
                author,
                reviewed_at: reviewed_at.map(|dt| dt.to_rfc3339()),
                issue_key: None,
                issue_summary: None,
            })
        }))
        .await;

        let mut out: Vec<ReviewedPR> = verified.into_iter().flatten().collect();
        // most recent review first; unknown-time entries sink to the bottom
        out.sort_by(|a, b| b.reviewed_at.cmp(&a.reviewed_at));
        Ok(out)
    }

    /// The local time of `login`'s latest review or issue comment on this PR
    /// today, if any (Comment/Approve/Request-changes all count).
    async fn last_action_today(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        login: &str,
        today: &str,
    ) -> Result<Option<DateTime<Local>>, String> {
        let mut latest: Option<DateTime<Local>> = None;
        let mut consider = |ts: &str| {
            if let Some(dt) = parse_local(ts) {
                if dt.format("%Y-%m-%d").to_string() == today
                    && latest.is_none_or(|cur| dt > cur)
                {
                    latest = Some(dt);
                }
            }
        };

        // `since` = local midnight — keeps today's comments in the first page
        // even on PRs with a long comment history
        let mut comments_q = vec![("per_page", "100".to_string())];
        if let Some(midnight) = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).single())
        {
            comments_q.push(("since", midnight.with_timezone(&Utc).to_rfc3339()));
        }
        // reviews and comments are independent endpoints — fetch both at once
        let reviews_url = format!("{API}/repos/{owner}/{repo}/pulls/{number}/reviews");
        let comments_url = format!("{API}/repos/{owner}/{repo}/issues/{number}/comments");
        let reviews_q = [("per_page", "100".to_string())];
        let (reviews, comments) = tokio::join!(
            self.get(&reviews_url, &reviews_q),
            self.get(&comments_url, &comments_q),
        );
        let (reviews, comments) = (reviews?, comments?);

        if let Some(arr) = reviews.as_array() {
            for r in arr {
                if r["user"]["login"].as_str() == Some(login) {
                    consider(r["submitted_at"].as_str().unwrap_or_default());
                }
            }
        }
        if let Some(arr) = comments.as_array() {
            for c in arr {
                if c["user"]["login"].as_str() == Some(login) {
                    consider(c["created_at"].as_str().unwrap_or_default());
                }
            }
        }
        Ok(latest)
    }
}

/// "https://api.github.com/repos/owner/name" → ("owner", "name")
fn parse_owner_repo(repository_url: &str) -> Option<(String, String)> {
    let rest = repository_url.strip_prefix("https://api.github.com/repos/")?;
    let mut it = rest.splitn(2, '/');
    let owner = it.next()?.to_string();
    let repo = it.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

/// Parse an RFC3339 GitHub timestamp into local time.
fn parse_local(ts: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Local))
}

fn api_err(status: reqwest::StatusCode, url: &str, body: &Value) -> String {
    let msg = body["message"].as_str().unwrap_or_default();
    let base = if msg.is_empty() {
        format!("GitHub ตอบกลับ {status} ที่ {url}")
    } else {
        format!("GitHub ตอบกลับ {status}: {msg}")
    };
    match status.as_u16() {
        401 => format!("{base}\n(401 = token ไม่ผ่าน — ลอง disconnect แล้ว Login GitHub ใหม่)"),
        403 => format!("{base}\n(403 = อาจติด rate limit หรือ SSO ของ org ยังไม่ได้ authorize token นี้)"),
        _ => base,
    }
}
