const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

let settings = null;
let issues = [];
let autoSuggestions = [];
let loggedKeys = new Set();
let timerInterval = null;
let targetHours = 8;

/* ================= helpers ================= */

function parseDuration(text) {
  const t = (text || "").trim().toLowerCase();
  if (!t) return 0;
  let m;
  if ((m = t.match(/^(\d{1,2}):(\d{1,2})$/))) return (+m[1] * 60 + +m[2]) * 60;
  if ((m = t.match(/^(\d+)\s*h\s*(\d+)\s*m?$/))) return +m[1] * 3600 + +m[2] * 60;
  if ((m = t.match(/^(\d+(?:\.\d+)?)\s*h$/))) return Math.round(+m[1] * 3600);
  if ((m = t.match(/^(\d+)\s*m$/))) return +m[1] * 60;
  if ((m = t.match(/^(\d+(?:\.\d+)?)$/))) return Math.round(+m[1] * 3600);
  return 0;
}

function fmtHM(secs) {
  const mins = Math.round(secs / 60);
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return `${h}:${String(m).padStart(2, "0")}`;
}

function fmtHours(secs) {
  const h = Math.round((secs / 3600) * 10) / 10;
  return Number.isInteger(h) ? String(h) : h.toFixed(1);
}

/* HH:MM:SS for the live timer readout (matches the Compact design) */
function fmtHMS(secs) {
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return [h, m, sec].map((n) => String(n).padStart(2, "0")).join(":");
}

function setMsg(id, text, ok) {
  const el = $(id);
  el.textContent = text;
  el.className = "msg " + (ok ? "ok" : "err");
  if (text) setTimeout(() => { if (el.textContent === text) el.textContent = ""; }, 6000);
}

/* escape for both text and attribute context (quotes included — titles/values
   are interpolated into attributes) */
function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => (
    { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]
  ));
}

/* ================= today summary ================= */

const RING_C = 2 * Math.PI * 25; // circumference of the summary ring (r = 25)

function renderToday(totalSecs) {
  $("today-done").textContent = totalSecs == null ? "–" : fmtHours(totalSecs);
  $("today-target").textContent = targetHours;
  const pct = totalSecs == null ? 0 : Math.min(100, (totalSecs / 3600 / targetHours) * 100);
  const ring = $("ring-fill");
  if (ring) ring.setAttribute("stroke-dasharray", `${((pct / 100) * RING_C).toFixed(2)} ${RING_C.toFixed(2)}`);
  const complete = totalSecs != null && totalSecs >= targetHours * 3600;
  if (ring) ring.classList.toggle("done", complete);
  const pctEl = $("ring-pct");
  if (pctEl) pctEl.textContent = totalSecs == null ? "–" : Math.round(pct) + "%";
  if (totalSecs != null) {
    const missing = targetHours * 3600 - totalSecs;
    $("today-note").innerHTML =
      missing > 0 ? `เหลืออีก <b>${fmtHours(missing)}</b> ชม.` : "ครบแล้ว 🎉";
  } else {
    $("today-note").textContent = "";
  }
}

async function refreshToday(force) {
  try {
    const total = await invoke("get_today_total", { force: !!force });
    renderToday(total);
  } catch (e) {
    renderToday(null);
    $("today-note").textContent = String(e);
  }
}

/* ================= issues ================= */

function issueLabel(i) {
  return `${i.key} — ${i.summary}`;
}

function fillIssueSelect(sel, selectedKey) {
  sel.innerHTML = '<option value="">— เลือก task —</option>';
  for (const i of issues) {
    const opt = document.createElement("option");
    opt.value = i.key;
    opt.textContent = issueLabel(i).slice(0, 80);
    if (i.key === selectedKey) opt.selected = true;
    sel.appendChild(opt);
  }
}

function setIssueStatus(text, kind) {
  const el = $("issue-status");
  if (!el) return;
  el.textContent = text || "";
  el.className = "issue-status" + (kind ? " " + kind : "");
}

async function loadIssues(force) {
  setIssueStatus("กำลังโหลด task จาก sprint…", "");
  try {
    const r = await invoke("get_sprint_issues", { force: !!force });
    issues = r.issues || [];
    // live-update the picker dropdown if the user has it open
    if (!$("issue-dropdown").hidden) showIssueList(filteredIssues($("issue-search").value));
    const sprint = r.sprint_name ? `sprint: ${r.sprint_name}` : "sprint ปัจจุบัน";
    if (!issues.length) {
      const proj = (settings && settings.jira_project_key) || "?";
      setIssueStatus(`ไม่พบ task ใน ${sprint} (project ${proj})`, "warn");
    } else {
      setIssueStatus(`${sprint} — ${issues.length} task`, "ok");
    }
  } catch (e) {
    // persistent — the error usually explains a 401/scope problem
    setIssueStatus(String(e), "err");
  }
}

/* ================= searchable task picker ================= */

let currentIssueKey = ""; // resolved selection (issue key)
let comboItems = [];      // issues currently rendered in the dropdown
let comboHl = -1;         // keyboard-highlighted index

function issueByKey(key) {
  const k = (key || "").trim().toUpperCase();
  return issues.find((i) => i.key.toUpperCase() === k) || null;
}

/* select an issue — a raw key outside the sprint is allowed (Jira accepts
   worklogs on any valid key, e.g. a backlog task from a branch chip) */
function setIssue(key) {
  const i = issueByKey(key);
  currentIssueKey = i ? i.key : (key || "").trim().toUpperCase();
  $("issue-search").value = i ? issueLabel(i) : currentIssueKey;
  hideIssueList();
}

function filteredIssues(q) {
  const t = (q || "").trim().toLowerCase();
  if (!t) return issues;
  return issues.filter(
    (i) => i.key.toLowerCase().includes(t) || i.summary.toLowerCase().includes(t)
  );
}

function showIssueList(list) {
  const box = $("issue-dropdown");
  box.innerHTML = "";
  comboItems = list;
  comboHl = -1;
  if (!list.length) {
    box.innerHTML = '<div class="combo-empty">ไม่พบ task ที่ตรงกับคำค้น</div>';
  } else {
    list.forEach((i) => {
      const div = document.createElement("div");
      div.className = "combo-item";
      div.innerHTML = `<b>${esc(i.key)}</b><span class="sum">${esc(i.summary)}</span>`;
      div.title = issueLabel(i);
      // mousedown (not click) — it fires before the input's blur hides the list
      div.onmousedown = (e) => { e.preventDefault(); setIssue(i.key); };
      box.appendChild(div);
    });
  }
  box.hidden = false;
}

function hideIssueList() {
  $("issue-dropdown").hidden = true;
  comboHl = -1;
}

function moveComboHl(dir) {
  const els = document.querySelectorAll("#issue-dropdown .combo-item");
  if (!els.length) return;
  comboHl = (comboHl + dir + els.length) % els.length;
  els.forEach((el, i) => el.classList.toggle("hl", i === comboHl));
  els[comboHl].scrollIntoView({ block: "nearest" });
}

/* what to log to: the picked issue, or the best match for the typed text */
function resolveIssueKey() {
  if (currentIssueKey) return currentIssueKey;
  const t = $("issue-search").value.trim();
  if (!t) return "";
  const exact = issueByKey(t.split(/[\s—]/)[0]);
  if (exact) { setIssue(exact.key); return exact.key; }
  const list = filteredIssues(t);
  if (list.length === 1) { setIssue(list[0].key); return list[0].key; }
  return "";
}

{
  const input = $("issue-search");
  input.addEventListener("focus", () => {
    input.select();
    showIssueList(issues);
  });
  input.addEventListener("input", () => {
    currentIssueKey = ""; // typing invalidates the previous pick
    showIssueList(filteredIssues(input.value));
  });
  input.addEventListener("blur", () => hideIssueList());
  input.addEventListener("keydown", (e) => {
    const open = !$("issue-dropdown").hidden;
    if (e.key === "Escape" && open) {
      hideIssueList();
      e.stopPropagation(); // don't let the global handler hide the window
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (!open) showIssueList(filteredIssues(input.value));
      moveComboHl(e.key === "ArrowDown" ? 1 : -1);
      e.preventDefault();
      return;
    }
    if (e.key === "Enter" && open && comboItems.length) {
      setIssue(comboItems[Math.max(0, comboHl)].key);
      e.preventDefault();
    }
  });
}

/* ================= branch suggestions ================= */

async function loadBranches() {
  const box = $("branch-chips");
  box.innerHTML = "";
  try {
    const branches = await invoke("get_branches");
    for (const b of branches.slice(0, 5)) {
      if (!b.issue_key) continue;
      const chip = document.createElement("button");
      chip.className = "chip";
      chip.innerHTML = `<b>${esc(b.issue_key)}</b> <span class="dim">· ${esc(b.repo)} (${esc(b.branch)})</span>`;
      chip.title = `${b.path}\nbranch: ${b.branch}`;
      chip.onclick = () => {
        setIssue(b.issue_key);
        if (!issueByKey(b.issue_key)) {
          setMsg("log-msg", `${b.issue_key} ไม่อยู่ใน sprint ปัจจุบัน — ยังลงเวลาได้ถ้า key ถูกต้อง`, true);
        }
      };
      box.appendChild(chip);
    }
  } catch { /* no roots configured */ }
}

/* ================= timer ================= */

function renderTimer(st) {
  const btn = $("btn-timer");
  const disp = $("timer-display");
  if (st.running) {
    btn.textContent = "⏹ หยุดจับเวลา";
    disp.className = "timer-display";
    disp.title = st.issue_key || "";
    let elapsed = st.elapsed_secs;
    disp.textContent = fmtHMS(elapsed);
    clearInterval(timerInterval);
    timerInterval = setInterval(() => {
      elapsed += 1;
      disp.textContent = fmtHMS(elapsed);
    }, 1000);
  } else {
    btn.textContent = "▶ เริ่มจับเวลา";
    disp.className = "timer-display idle";
    disp.textContent = "";
    clearInterval(timerInterval);
  }
}

async function toggleTimer() {
  try {
    const st = await invoke("timer_status");
    if (st.running) {
      const stopped = await invoke("timer_stop");
      renderTimer({ running: false });
      // prefill the form with elapsed time
      setIssue(stopped.issue_key || "");
      $("time-input").value = fmtHM(Math.max(60, stopped.elapsed_secs));
      setMsg("log-msg", `จับเวลาได้ ${fmtHM(stopped.elapsed_secs)} — กดบันทึกเพื่อลงเวลา`, true);
    } else {
      const key = resolveIssueKey();
      if (!key) return setMsg("log-msg", "เลือก task ก่อนเริ่มจับเวลา", false);
      const started = await invoke("timer_start", { issueKey: key });
      renderTimer(started);
    }
  } catch (e) {
    setMsg("log-msg", String(e), false);
  }
}

/* ================= log work ================= */

async function submitLog() {
  const key = resolveIssueKey();
  const secs = parseDuration($("time-input").value);
  if (!key) return setMsg("log-msg", "เลือก task ก่อน", false);
  if (!secs) return setMsg("log-msg", "ใส่เวลาให้ถูก เช่น 1:30 หรือ 1.5", false);
  const btn = $("btn-log");
  btn.disabled = true;
  try {
    const total = await invoke("log_work", {
      issueKey: key,
      seconds: secs,
      comment: $("comment-input").value,
      started: null,
    });
    renderToday(total);
    $("time-input").value = "";
    $("comment-input").value = "";
    setMsg("log-msg", `ลงเวลา ${fmtHM(secs)} ที่ ${key} แล้ว ✓`, true);
  } catch (e) {
    setMsg("log-msg", String(e), false);
  } finally {
    btn.disabled = false;
  }
}

/* ================= auto tab ================= */

/* stable per-item key used to remember what was already logged today */
function logKeyOf(s) {
  return s.source === "github"
    ? `pr:${s.url}`
    : `cal:${s.event.summary}|${s.event.start || ""}`;
}

function renderAutoList() {
  const box = $("auto-list");
  box.innerHTML = "";
  if (!autoSuggestions.length) {
    box.innerHTML = '<div class="hint">ไม่พบนัดใน calendar หรือ PR ที่ review วันนี้</div>';
    $("btn-auto-confirm").style.display = "none";
    return;
  }
  let anyPending = false;
  autoSuggestions.forEach((s, idx) => {
    const div = document.createElement("div");
    const isGh = s.source === "github";
    const done = loggedKeys.has(logKeyOf(s));
    if (!done) anyPending = true;
    div.className =
      "auto-item" +
      (s.issue_key ? "" : " unmatched") +
      (isGh ? " github" : "") +
      (done ? " logged" : "");
    const timeTxt = s.event.all_day
      ? (isGh
          ? (s.reviewed_at ? `รีวิว ${s.reviewed_at.slice(11, 16)}` : "review")
          : "ทั้งวัน")
      : `${s.event.start.slice(11, 16)}–${s.event.end.slice(11, 16)}`;
    const durDefault = s.event.duration_secs ? fmtHM(s.event.duration_secs) : "1:00";
    const badge =
      (done ? '<span class="done-badge">ลงแล้ว ✓</span>' : "") +
      (isGh ? '<span class="src-badge">PR</span>' : "");
    const dis = done ? "disabled" : "";
    div.innerHTML = `
      <div class="top">
        <input type="checkbox" data-idx="${idx}" ${done ? "" : s.issue_key ? "checked" : ""} ${dis} />
        ${badge}
        <span class="ev-title" title="${esc(s.event.summary)}">${esc(s.event.summary)}</span>
        <span class="ev-time">${timeTxt}</span>
      </div>
      <div class="map-row">
        <select data-idx="${idx}" class="auto-issue" ${dis}></select>
        <input data-idx="${idx}" class="auto-dur" value="${durDefault}" title="เวลา เช่น 1:30" ${dis} />
      </div>`;
    box.appendChild(div);
    fillIssueSelect(div.querySelector("select"), s.issue_key || "");
  });
  // nothing left to log → hide the confirm button
  $("btn-auto-confirm").style.display = anyPending ? "block" : "none";
  updateConfirmSummary();
}

/* reflect the current selection on the confirm button: "2 รายการ · รวม 1:30" */
function updateConfirmSummary() {
  const btn = $("btn-auto-confirm");
  let count = 0;
  let total = 0;
  document.querySelectorAll("#auto-list .auto-item").forEach((item) => {
    if (item.classList.contains("logged")) return;
    const cb = item.querySelector('input[type="checkbox"]');
    if (!cb || !cb.checked) return;
    const secs = parseDuration(item.querySelector(".auto-dur").value);
    if (!item.querySelector("select").value || !secs) return;
    count++;
    total += secs;
  });
  btn.textContent = count
    ? `ยืนยันลงเวลา ${count} รายการ · รวม ${fmtHM(total)} ชม.`
    : "ยืนยันลงเวลาที่เลือก";
}

async function fetchAuto() {
  $("auto-list").innerHTML = '<div class="hint">กำลังโหลด…</div>';
  autoSuggestions = [];
  let err = "";
  // Google Calendar events
  try {
    const cal = await invoke("get_auto_suggestions");
    cal.forEach((c) => autoSuggestions.push({ ...c, source: "calendar" }));
  } catch (e) {
    err = String(e);
  }
  // GitHub PRs reviewed today — normalize into the same auto-suggestion shape
  // so renderAutoList()/confirmAuto() handle them without special-casing
  try {
    const prs = await invoke("get_reviewed_prs");
    prs.forEach((pr) =>
      autoSuggestions.push({
        source: "github",
        url: pr.url,
        reviewed_at: pr.reviewed_at,
        issue_key: pr.issue_key,
        issue_summary: pr.issue_summary,
        event: {
          summary: `Review ${pr.repo}#${pr.number}: ${pr.title}`,
          all_day: true,
          start: null,
          end: null,
          duration_secs: 1800,
        },
      })
    );
  } catch (e) {
    const msg = String(e);
    // "not connected" is expected when GitHub is unused — don't nag about it
    if (!msg.includes("ยังไม่ได้เชื่อมต่อ GitHub")) {
      err = err ? `${err} | ${msg}` : msg;
    }
  }
  // which of these were already logged today? (survives re-fetch/restart)
  try {
    loggedKeys = new Set(await invoke("get_logged_keys"));
  } catch {
    loggedKeys = new Set();
  }
  renderAutoList();
  if (err) setMsg("auto-msg", err, false);
}

async function confirmAuto() {
  const entries = [];
  document.querySelectorAll("#auto-list .auto-item").forEach((item) => {
    if (item.classList.contains("logged")) return; // already logged today
    const cb = item.querySelector('input[type="checkbox"]');
    if (!cb.checked) return;
    const idx = +cb.dataset.idx;
    const key = item.querySelector("select").value;
    const secs = parseDuration(item.querySelector(".auto-dur").value);
    if (!key || !secs) return;
    const s = autoSuggestions[idx];
    entries.push({
      issue_key: key,
      seconds: secs,
      comment: s.event.summary,
      started: s.event.all_day ? null : toJiraTs(s.event.start),
      key: logKeyOf(s),
    });
  });
  if (!entries.length) return setMsg("auto-msg", "ไม่มีรายการที่เลือก/ใส่เวลาครบ", false);
  const btn = $("btn-auto-confirm");
  btn.disabled = true;
  try {
    const total = await invoke("confirm_auto", { entries });
    renderToday(total);
    setMsg("auto-msg", `ลงเวลา ${entries.length} รายการแล้ว ✓`, true);
  } catch (e) {
    setMsg("auto-msg", String(e), false);
    refreshToday(true);
  } finally {
    btn.disabled = false;
    // refresh "logged" markers from the backend (source of truth, even on
    // partial failure) and re-render so logged items get badged in place
    try {
      loggedKeys = new Set(await invoke("get_logged_keys"));
    } catch {}
    renderAutoList();
  }
}

// "2026-07-05T10:00:00+07:00" → "2026-07-05T10:00:00.000+0700"
function toJiraTs(rfc3339) {
  const m = rfc3339.match(/^(.+?)(?:\.\d+)?([+-]\d{2}):(\d{2})$/);
  if (!m) return null;
  return `${m[1]}.000${m[2]}${m[3]}`;
}

/* ================= settings ================= */

function renderRules(rules) {
  const box = $("rules-list");
  box.innerHTML = "";
  rules.forEach((r, i) => {
    const row = document.createElement("div");
    row.className = "rule-row";
    row.innerHTML = `
      <input class="rule-cal" placeholder="คำใน calendar เช่น Mandrake Grooming" value="${esc(r.calendar_keyword)}" />
      <input class="rule-jira" placeholder="prefix task เช่น Grooming" value="${esc(r.jira_prefix)}" />
      <button class="del" title="ลบ">✕</button>`;
    row.querySelector(".del").onclick = () => { row.remove(); };
    box.appendChild(row);
  });
}

function readRules() {
  return [...document.querySelectorAll("#rules-list .rule-row")]
    .map((row) => ({
      calendar_keyword: row.querySelector(".rule-cal").value.trim(),
      jira_prefix: row.querySelector(".rule-jira").value.trim(),
    }))
    .filter((r) => r.calendar_keyword && r.jira_prefix);
}

function fillSettingsForm(s) {
  $("s-jira-url").value = s.jira_base_url;
  $("s-jira-email").value = s.jira_email;
  $("s-jira-token").value = s.jira_api_token;
  $("s-jira-project").value = s.jira_project_key;
  $("s-atl-id").value = s.atlassian_client_id || "";
  $("s-atl-secret").value = s.atlassian_client_secret || "";
  $("s-goog-id").value = s.google_client_id || "";
  $("s-goog-secret").value = s.google_client_secret || "";
  $("s-github-org").value = s.github_org || "";
  $("s-gh-id").value = s.github_client_id || "";
  $("s-gh-secret").value = s.github_client_secret || "";
  $("s-hours").value = s.work_hours_per_day;
  $("s-remind-every").value = s.remind_every_minutes;
  $("s-remind-before").value = s.remind_before_end_minutes;
  $("s-eod").value = s.end_of_day;
  $("s-refresh").value = s.refresh_interval_minutes;
  $("s-hotkey").value = s.hotkey;
  $("s-roots").value = (s.workspace_roots || []).join("\n");
  $("s-ics").value = s.ics_url;
  renderRules(s.auto_rules || []);
  const eodEl = $("today-eod");
  if (eodEl) {
    eodEl.textContent = s.end_of_day || "";
    $("sum-eod").hidden = !s.end_of_day;
  }
}

async function loadSettings() {
  settings = await invoke("get_settings");
  targetHours = settings.work_hours_per_day;
  fillSettingsForm(settings);
}

async function saveSettings() {
  const ns = {
    jira_base_url: $("s-jira-url").value.trim(),
    jira_email: $("s-jira-email").value.trim(),
    jira_api_token: $("s-jira-token").value.trim(),
    jira_project_key: $("s-jira-project").value.trim() || "MDT",
    work_hours_per_day: +$("s-hours").value || 8,
    remind_every_minutes: Math.max(0, +$("s-remind-every").value || 0),
    remind_before_end_minutes: Math.max(0, +$("s-remind-before").value || 0),
    end_of_day: $("s-eod").value.trim() || "18:00",
    refresh_interval_minutes: Math.max(1, +$("s-refresh").value || 15),
    hotkey: $("s-hotkey").value.trim(),
    workspace_roots: $("s-roots").value.split("\n").map((s) => s.trim()).filter(Boolean),
    ics_url: $("s-ics").value.trim(),
    auto_rules: readRules(),
    atlassian_client_id: $("s-atl-id").value.trim(),
    atlassian_client_secret: $("s-atl-secret").value.trim(),
    google_client_id: $("s-goog-id").value.trim(),
    google_client_secret: $("s-goog-secret").value.trim(),
    github_org: $("s-github-org").value.trim(),
    github_client_id: $("s-gh-id").value.trim(),
    github_client_secret: $("s-gh-secret").value.trim(),
  };
  try {
    await invoke("save_settings", { newSettings: ns });
    settings = ns;
    targetHours = ns.work_hours_per_day;
    renderToday(null);
    setMsg("settings-msg", "บันทึกแล้ว ✓", true);
    loadIssues(true);
    refreshToday(true);
  } catch (e) {
    setMsg("settings-msg", String(e), false);
  }
}

/* ================= oauth connections ================= */

function renderConnStatus(st) {
  const jiraEl = $("jira-status");
  const jiraBtn = $("btn-conn-jira");
  if (st.atlassian_connected) {
    jiraEl.textContent = `เชื่อมต่อแล้ว ✓ ${st.atlassian_site || ""}`;
    jiraEl.className = "hint connected";
    jiraBtn.textContent = "ยกเลิกการเชื่อมต่อ";
  } else {
    jiraEl.textContent = "ยังไม่ได้เชื่อมต่อ";
    jiraEl.className = "hint";
    jiraBtn.textContent = "Login ด้วย Atlassian";
  }
  const gEl = $("google-status");
  const gBtn = $("btn-conn-google");
  if (st.google_connected) {
    gEl.textContent = `เชื่อมต่อแล้ว ✓ ${st.google_email || ""}`;
    gEl.className = "hint connected";
    gBtn.textContent = "ยกเลิกการเชื่อมต่อ";
  } else {
    gEl.textContent = "ยังไม่ได้เชื่อมต่อ";
    gEl.className = "hint";
    gBtn.textContent = "Login ด้วย Google";
  }

  const ghEl = $("github-status");
  const ghBtn = $("btn-conn-github");
  if (st.github_connected) {
    ghEl.textContent = `เชื่อมต่อแล้ว ✓ ${st.github_login || ""}`;
    ghEl.className = "hint connected";
    ghBtn.textContent = "ยกเลิกการเชื่อมต่อ";
  } else {
    ghEl.textContent = "ยังไม่ได้เชื่อมต่อ";
    ghEl.className = "hint";
    ghBtn.textContent = "Login ด้วย GitHub";
  }

  // when credentials are baked into the build, there's nothing to configure —
  // hide the manual fallbacks and the whole "OAuth App" section
  const allEmbedded = st.atlassian_embedded && st.google_embedded && st.github_embedded;
  const jiraManual = $("jira-manual");
  if (jiraManual) jiraManual.style.display = st.atlassian_embedded ? "none" : "";
  const googleManual = $("google-manual");
  if (googleManual) googleManual.style.display = st.google_embedded ? "none" : "";
  const oauthSection = $("oauth-app-section");
  if (oauthSection) oauthSection.style.display = allEmbedded ? "none" : "";
}

let connStatus = { atlassian_connected: false, google_connected: false, github_connected: false };

async function loadConnStatus() {
  try {
    connStatus = await invoke("connection_status");
    renderConnStatus(connStatus);
  } catch { /* ignore */ }
}

async function loadProjects() {
  try {
    const projects = await invoke("list_projects");
    const dl = $("project-list");
    dl.innerHTML = "";
    for (const p of projects) {
      const opt = document.createElement("option");
      opt.value = p.key;
      opt.label = `${p.key} — ${p.name}`;
      dl.appendChild(opt);
    }
  } catch { /* not connected yet */ }
}

async function toggleConnection(provider, btnId, msgId) {
  const btn = $(btnId);
  const connected =
    provider === "atlassian"
      ? connStatus.atlassian_connected
      : provider === "google"
        ? connStatus.google_connected
        : connStatus.github_connected;
  btn.disabled = true;
  try {
    if (connected) {
      connStatus = await invoke("disconnect_provider", { provider });
      renderConnStatus(connStatus);
      setMsg(msgId, "ยกเลิกการเชื่อมต่อแล้ว", true);
    } else {
      // client id/secret in the form must be saved before starting the flow
      await saveSettings();
      btn.textContent = "รอ login ในเบราว์เซอร์…";
      connStatus = await invoke("connect_provider", { provider });
      renderConnStatus(connStatus);
      setMsg(msgId, "เชื่อมต่อสำเร็จ ✓", true);
      if (provider === "atlassian") {
        loadProjects();
        loadIssues(true);
        refreshToday(true);
      }
    }
  } catch (e) {
    renderConnStatus(connStatus);
    setMsg(msgId, String(e), false);
  } finally {
    btn.disabled = false;
  }
}

/* ================= overlays & events ================= */

/* Auto & Settings live in header-triggered overlays (Compact — no tabs) */
function openOverlay(name) {
  const el = $("overlay-" + name);
  if (!el) return;
  el.hidden = false;
  if (name === "auto" && !autoSuggestions.length) fetchAuto();
}
function closeOverlays() {
  document.querySelectorAll(".overlay").forEach((el) => (el.hidden = true));
}
function anyOverlayOpen() {
  return [...document.querySelectorAll(".overlay")].some((el) => !el.hidden);
}
$("btn-open-auto").onclick = () => openOverlay("auto");
$("btn-open-settings").onclick = () => openOverlay("settings");
$("btn-auto-back").onclick = closeOverlays;
$("btn-settings-back").onclick = closeOverlays;

/* collapsible "เพิ่มรายละเอียด" (comment) */
$("btn-details").onclick = () => {
  const open = $("comment-wrap").classList.toggle("open");
  $("btn-details").classList.toggle("open", open);
  if (open) $("comment-input").focus();
};

$("btn-close").onclick = () => invoke("hide_window");

/* pin = keep the widget on screen (don't auto-hide on blur) */
function renderPin(pinned) {
  const btn = $("btn-pin");
  btn.classList.toggle("active", pinned);
  btn.title = pinned
    ? "เลิกปักหมุด — กลับไปซ่อนอัตโนมัติเมื่อคลิกที่อื่น"
    : "ปักหมุด — ค้างไว้บนจอ ไม่ซ่อนอัตโนมัติ";
}
$("btn-pin").onclick = async () => {
  try {
    const pinned = await invoke("toggle_pin");
    renderPin(pinned);
  } catch (e) {
    setMsg("log-msg", String(e), false);
  }
};
invoke("get_pinned").then(renderPin).catch(() => {});
$("btn-refresh").onclick = () => { refreshToday(true); loadIssues(true); loadBranches(); };
$("btn-log").onclick = submitLog;
$("btn-timer").onclick = toggleTimer;
$("btn-auto-fetch").onclick = fetchAuto;
$("btn-auto-confirm").onclick = confirmAuto;
/* Enter in the time field = save (comment is optional anyway) */
$("time-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") submitLog();
});
/* live-update the confirm button as checkboxes / durations / tasks change */
$("auto-list").addEventListener("change", updateConfirmSummary);
$("auto-list").addEventListener("input", updateConfirmSummary);
$("btn-save-settings").onclick = saveSettings;
$("btn-diagnose").onclick = async () => {
  const out = $("diagnose-out");
  out.style.display = "block";
  out.textContent = "กำลังทดสอบ…";
  try {
    out.textContent = await invoke("diagnose_jira");
  } catch (e) {
    out.textContent = String(e);
  }
};
$("btn-conn-jira").onclick = () => toggleConnection("atlassian", "btn-conn-jira", "settings-msg");
$("btn-conn-google").onclick = () => toggleConnection("google", "btn-conn-google", "settings-msg");
$("btn-conn-github").onclick = () => toggleConnection("github", "btn-conn-github", "settings-msg");
$("btn-add-rule").onclick = () => {
  const rules = readRules();
  rules.push({ calendar_keyword: "", jira_prefix: "" });
  renderRules(rules);
};

/* ---- autostart + updates (apply immediately, not tied to the save button) ---- */

async function loadGeneralInfo() {
  try { $("app-version").textContent = "v" + (await invoke("app_version")); } catch {}
  try { $("s-autostart").checked = await invoke("get_autostart"); } catch {}
}

$("s-autostart").onchange = async (e) => {
  const want = e.target.checked;
  try {
    await invoke("set_autostart", { enabled: want });
    setMsg("settings-msg", want ? "เปิดอัตโนมัติตอน login แล้ว ✓" : "ปิด autostart แล้ว", true);
  } catch (err) {
    e.target.checked = !want; // revert the toggle — the OS call failed
    setMsg("settings-msg", String(err), false);
  }
};

$("btn-check-update").onclick = async () => {
  const st = $("update-status");
  const btn = $("btn-check-update");
  btn.disabled = true;
  st.textContent = "· กำลังตรวจ…";
  try {
    const info = await invoke("check_update");
    if (!info) {
      st.textContent = "· เป็นเวอร์ชันล่าสุดแล้ว ✓";
      return;
    }
    st.textContent = `· พบ v${info.version} — กำลังดาวน์โหลด+ติดตั้ง…`;
    await invoke("install_update"); // แอปจะปิดตัว/รีสตาร์ตเองเมื่อเสร็จ
  } catch (e) {
    st.textContent = "";
    setMsg("settings-msg", String(e), false);
  } finally {
    btn.disabled = false;
  }
};

document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (anyOverlayOpen()) { closeOverlays(); return; }
  invoke("hide_window");
});

/* tell the backend a drag is happening so it doesn't auto-hide mid-drag.
   begin: mousedown on a drag region. end: mouseup (a plain click) or the
   window regaining focus after the drop (handled in Rust on Focused(true)). */
document.addEventListener("mousedown", (e) => {
  if (e.button === 0 && e.target.closest("[data-tauri-drag-region]")) {
    invoke("begin_drag");
  }
});
window.addEventListener("mouseup", () => invoke("end_drag"));

/* refresh data every time the panel opens */
listen("panel-shown", () => {
  refreshToday(false);
  loadIssues(false);
  loadBranches();
  invoke("timer_status").then(renderTimer).catch(() => {});
});

listen("total-updated", (ev) => renderToday(ev.payload));

/* ================= init ================= */

(async function init() {
  await loadSettings();
  renderToday(null);
  refreshToday(false);
  loadIssues(false);
  loadBranches();
  loadConnStatus();
  loadProjects();
  loadGeneralInfo();
  invoke("timer_status").then(renderTimer).catch(() => {});
})();
