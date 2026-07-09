use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings::Settings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLite {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub assignee: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLite {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkType {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintLite {
    pub id: u64,
    pub name: String,
    pub state: String,
}

enum JiraAuth {
    Basic { email: String, token: String },
    Bearer(String),
}

pub struct JiraClient {
    http: reqwest::Client,
    base: String,
    auth: JiraAuth,
}

impl JiraClient {
    fn build(base: String, auth: JiraAuth) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { http, base, auth })
    }

    pub fn from_settings(s: &Settings) -> Result<Self, String> {
        if s.jira_base_url.is_empty() || s.jira_email.is_empty() || s.jira_api_token.is_empty() {
            return Err("Jira ยังไม่ได้ตั้งค่า (base url / email / api token)".into());
        }
        Self::build(
            s.jira_base_url.trim_end_matches('/').to_string(),
            JiraAuth::Basic {
                email: s.jira_email.clone(),
                token: s.jira_api_token.clone(),
            },
        )
    }

    /// OAuth client hitting https://api.atlassian.com/ex/jira/{cloud_id}
    pub fn bearer(base: String, access_token: String) -> Result<Self, String> {
        Self::build(base, JiraAuth::Bearer(access_token))
    }

    fn apply_auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            JiraAuth::Basic { email, token } => rb.basic_auth(email, Some(token)),
            JiraAuth::Bearer(t) => rb.bearer_auth(t),
        }
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        let res = self
            .apply_auth(self.http.get(format!("{}{}", self.base, path)))
            .query(query)
            .send()
            .await
            .map_err(|e| format!("Jira request ล้มเหลว: {e}"))?;
        let status = res.status();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(api_err(status, path, &body));
        }
        Ok(body)
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let res = self
            .apply_auth(self.http.post(format!("{}{}", self.base, path)))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Jira request ล้มเหลว: {e}"))?;
        let status = res.status();
        let resp: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(api_err(status, path, &resp));
        }
        Ok(resp)
    }

    /// Search issues with the (new) JQL search endpoint.
    async fn search(&self, jql: &str, fields: &[&str]) -> Result<Vec<Value>, String> {
        let body = json!({
            "jql": jql,
            "maxResults": 100,
            "fields": fields,
        });
        let res = self.post("/rest/api/3/search/jql", body).await?;
        Ok(res["issues"].as_array().cloned().unwrap_or_default())
    }

    fn issues_from(values: &[Value]) -> Vec<IssueLite> {
        values
            .iter()
            .map(|i| IssueLite {
                key: i["key"].as_str().unwrap_or_default().to_string(),
                summary: i["fields"]["summary"].as_str().unwrap_or_default().to_string(),
                status: i["fields"]["status"]["name"].as_str().unwrap_or_default().to_string(),
                assignee: i["fields"]["assignee"]["displayName"].as_str().map(|s| s.to_string()),
            })
            .collect()
    }

    /// Id of the greenhopper "Sprint" custom field (e.g. customfield_10020).
    /// Some sites have several fields literally named "Sprint"; only the
    /// gh-sprint one carries real sprint objects, and the JQL `openSprints()`
    /// keyword can resolve to the wrong one — so we always read this field id.
    /// Stable per site — callers cache it in AppState.
    pub async fn sprint_field_id(&self) -> Result<String, String> {
        let fields = self.get("/rest/api/3/field", &[]).await?;
        fields
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|f| {
                        f["schema"]["custom"].as_str()
                            == Some("com.pyxis.greenhopper.jira:gh-sprint")
                    })
                    .and_then(|f| f["id"].as_str())
            })
            .map(|s| s.to_string())
            .ok_or_else(|| "หา Sprint field (gh-sprint) ไม่เจอ".to_string())
    }

    /// Find the current sprint (id, name) by reading the sprint field of the
    /// project's most recently updated issues — active preferred, else newest.
    /// This sidesteps `openSprints()` (unreliable on sites with duplicate
    /// "Sprint" fields) and the Agile API (needs jira-software scopes).
    async fn current_sprint(
        &self,
        project_key: &str,
        sprint_field: &str,
    ) -> Result<Option<(u64, String)>, String> {
        let jql = format!("project = {project_key} ORDER BY updated DESC");
        let issues = self.search(&jql, &[sprint_field]).await?;
        let mut active: Option<(u64, String)> = None;
        let mut newest: Option<(u64, String)> = None;
        for iss in &issues {
            let Some(sprints) = iss["fields"][sprint_field].as_array() else {
                continue;
            };
            for sp in sprints {
                let Some(id) = sp["id"].as_u64() else { continue };
                let name = sp["name"].as_str().unwrap_or_default().to_string();
                let better = |cur: &Option<(u64, String)>| cur.as_ref().is_none_or(|(c, _)| id > *c);
                if sp["state"].as_str() == Some("active") && better(&active) {
                    active = Some((id, name.clone()));
                }
                if better(&newest) {
                    newest = Some((id, name));
                }
            }
        }
        Ok(active.or(newest))
    }

    /// Issues in the project's current sprint. Returns (sprint_name, issues).
    /// `sprint_field` comes from `sprint_field_id()` (cached by the caller).
    pub async fn sprint_issues(
        &self,
        project_key: &str,
        sprint_field: &str,
    ) -> Result<(Option<String>, Vec<IssueLite>), String> {
        let Some((sprint_id, name)) = self.current_sprint(project_key, sprint_field).await? else {
            return Ok((None, vec![])); // no sprint found on recent issues
        };
        // query by numeric sprint id — robust against names with quotes/spaces.
        // statusCategory != Done drops completed work (any "done"-type status).
        // ORDER BY Rank ASC = the manual board/backlog order
        let jql = format!(
            "project = {project_key} AND sprint = {sprint_id} AND statusCategory != Done ORDER BY Rank ASC"
        );
        let v = self
            .search(&jql, &["summary", "status", "assignee", "issuetype"])
            .await?;
        // keep tasks + sub-tasks (hierarchyLevel <= 0); drop epics (level 1+)
        let work_items: Vec<Value> = v
            .into_iter()
            .filter(|i| {
                i["fields"]["issuetype"]["hierarchyLevel"]
                    .as_i64()
                    .is_none_or(|l| l <= 0)
            })
            .collect();
        Ok((
            Some(format!("{name} ({})", work_items.len())),
            Self::issues_from(&work_items),
        ))
    }

    /// Projects visible to the user — for the settings dropdown.
    pub async fn list_projects(&self) -> Result<Vec<ProjectLite>, String> {
        let res = self
            .get(
                "/rest/api/3/project/search",
                &[("maxResults", "100".into()), ("orderBy", "key".into())],
            )
            .await?;
        Ok(res["values"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|p| ProjectLite {
                key: p["key"].as_str().unwrap_or_default().to_string(),
                name: p["name"].as_str().unwrap_or_default().to_string(),
            })
            .collect())
    }

    pub async fn my_account_id(&self) -> Result<String, String> {
        let me = self.get("/rest/api/3/myself", &[]).await?;
        me["accountId"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "อ่าน accountId ไม่ได้".into())
    }

    /// Total seconds logged by the current user today (local date).
    /// `account_id` comes from `my_account_id()` (cached by the caller).
    pub async fn today_logged_seconds(&self, account_id: &str) -> Result<u64, String> {
        let now = Local::now();
        let today = now.format("%Y-%m-%d").to_string();
        // local midnight in epoch millis — trims each worklog fetch to today's
        // entries instead of pulling an issue's entire history
        let started_after = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).single())
            .map(|t| t.timestamp_millis())
            .unwrap_or(0);
        let jql = format!(
            "worklogAuthor = currentUser() AND worklogDate = \"{today}\""
        );
        let issues = self.search(&jql, &["summary"]).await?;
        let mut total: u64 = 0;
        for issue in &issues {
            let key = issue["key"].as_str().unwrap_or_default();
            if key.is_empty() {
                continue;
            }
            let wl = self
                .get(
                    &format!("/rest/api/3/issue/{key}/worklog"),
                    &[
                        ("maxResults", "5000".into()),
                        ("startedAfter", started_after.to_string()),
                    ],
                )
                .await?;
            for w in wl["worklogs"].as_array().unwrap_or(&vec![]) {
                let author_ok = w["author"]["accountId"].as_str() == Some(account_id);
                let started = w["started"].as_str().unwrap_or_default();
                // started looks like 2026-07-05T10:00:00.000+0700 — compare local date prefix
                let date_ok = local_date_of(started).as_deref() == Some(today.as_str());
                if author_ok && date_ok {
                    total += w["timeSpentSeconds"].as_u64().unwrap_or(0);
                }
            }
        }
        Ok(total)
    }

    /// Log work on an issue. `started` is an RFC3339-ish Jira timestamp; if None, now.
    pub async fn log_work(
        &self,
        issue_key: &str,
        seconds: u64,
        comment: &str,
        started: Option<String>,
    ) -> Result<(), String> {
        let started = started.unwrap_or_else(|| {
            Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string()
        });
        let mut body = json!({
            "timeSpentSeconds": seconds,
            "started": started,
        });
        if !comment.trim().is_empty() {
            body["comment"] = adf(comment);
        }
        self.post(&format!("/rest/api/3/issue/{issue_key}/worklog"), body)
            .await?;
        Ok(())
    }

    /// Standard issue types available when creating an issue in `project_key`
    /// (Task / Bug / Support / …). Drops sub-tasks and epics so the list matches
    /// what the Create-Task form should offer. Falls back to a fixed list if the
    /// createmeta call fails or returns nothing usable.
    pub async fn issue_types(&self, project_key: &str) -> Result<Vec<WorkType>, String> {
        let fallback = || {
            ["Task", "Bug", "Support"]
                .iter()
                .map(|n| WorkType { id: String::new(), name: n.to_string() })
                .collect::<Vec<_>>()
        };
        let res = match self
            .get(
                &format!("/rest/api/3/issue/createmeta/{project_key}/issuetypes"),
                &[],
            )
            .await
        {
            Ok(v) => v,
            Err(_) => return Ok(fallback()),
        };
        let types: Vec<WorkType> = res["values"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter(|t| {
                        t["subtask"].as_bool() != Some(true)
                            && t["hierarchyLevel"].as_i64().is_none_or(|l| l == 0)
                    })
                    .filter_map(|t| {
                        let name = t["name"].as_str()?.to_string();
                        Some(WorkType {
                            id: t["id"].as_str().unwrap_or_default().to_string(),
                            name,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(if types.is_empty() { fallback() } else { types })
    }

    /// Selectable sprints (active + future) for the Create-Task form, read from
    /// the sprint field of the project's most recently updated issues — active
    /// first, then future by id. Same approach as `current_sprint()` (avoids the
    /// Agile API, which is unreliable on this site). Note: a future sprint only
    /// appears here once at least one issue is assigned to it.
    pub async fn list_sprints(
        &self,
        project_key: &str,
        sprint_field: &str,
    ) -> Result<Vec<SprintLite>, String> {
        let jql = format!("project = {project_key} ORDER BY updated DESC");
        let issues = self.search(&jql, &[sprint_field]).await?;
        let mut seen = std::collections::HashSet::new();
        let mut sprints: Vec<SprintLite> = Vec::new();
        for iss in &issues {
            let Some(list) = iss["fields"][sprint_field].as_array() else {
                continue;
            };
            for sp in list {
                let Some(id) = sp["id"].as_u64() else { continue };
                let state = sp["state"].as_str().unwrap_or_default().to_string();
                if state != "active" && state != "future" {
                    continue;
                }
                if seen.insert(id) {
                    sprints.push(SprintLite {
                        id,
                        name: sp["name"].as_str().unwrap_or_default().to_string(),
                        state,
                    });
                }
            }
        }
        // active before future; within each group, lower id first
        sprints.sort_by(|a, b| {
            let rank = |s: &str| if s == "active" { 0 } else { 1 };
            rank(&a.state).cmp(&rank(&b.state)).then(a.id.cmp(&b.id))
        });
        Ok(sprints)
    }

    /// Create an issue and return its new key (e.g. "MDT-123"). `sprint_id`, when
    /// given, is written to the gh-sprint custom field so the issue lands in that
    /// sprint. `issue_type_id` is preferred; if empty, falls back to type by name.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_issue(
        &self,
        project_key: &str,
        issue_type_id: &str,
        issue_type_name: &str,
        summary: &str,
        description: &str,
        sprint_id: Option<u64>,
        sprint_field: &str,
    ) -> Result<String, String> {
        let issuetype = if issue_type_id.is_empty() {
            json!({ "name": issue_type_name })
        } else {
            json!({ "id": issue_type_id })
        };
        let mut fields = json!({
            "project": { "key": project_key },
            "summary": summary.trim(),
            "issuetype": issuetype,
        });
        if !description.trim().is_empty() {
            fields["description"] = adf(description);
        }
        if let Some(sid) = sprint_id {
            fields[sprint_field] = json!(sid);
        }
        let resp = self.post("/rest/api/3/issue", json!({ "fields": fields })).await?;
        resp["key"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "สร้าง issue สำเร็จแต่อ่าน key ไม่ได้".to_string())
    }
}

/// Wrap plain text in a minimal Atlassian Document Format doc (single paragraph).
fn adf(text: &str) -> Value {
    json!({
        "type": "doc",
        "version": 1,
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text.trim() }]
        }]
    })
}

/// Convert a Jira "started" timestamp to its local-date string (YYYY-MM-DD).
fn local_date_of(started: &str) -> Option<String> {
    let dt = chrono::DateTime::parse_from_str(started, "%Y-%m-%dT%H:%M:%S%.3f%z").ok()?;
    Some(dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn api_err(status: reqwest::StatusCode, path: &str, body: &Value) -> String {
    let detail = short_err(body);
    let base = if detail.is_empty() {
        format!("Jira ตอบกลับ {status} ที่ {path}")
    } else {
        format!("Jira ตอบกลับ {status} ที่ {path}: {detail}")
    };
    match status.as_u16() {
        401 => format!(
            "{base}\n(401 = token ไม่ผ่าน — ลอง disconnect แล้ว Login ใหม่ และเช็คว่า OAuth app เปิด scope: read:jira-work, read:jira-user, write:jira-work)"
        ),
        403 => format!("{base}\n(403 = สิทธิ์ไม่พอ — เช็ค scope ของ OAuth app)"),
        _ => base,
    }
}

fn short_err(body: &Value) -> String {
    if let Some(msgs) = body["errorMessages"].as_array() {
        if !msgs.is_empty() {
            return msgs
                .iter()
                .filter_map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join(", ");
        }
    }
    if let Some(errs) = body["errors"].as_object() {
        if !errs.is_empty() {
            return serde_json::to_string(errs).unwrap_or_default();
        }
    }
    String::new()
}
