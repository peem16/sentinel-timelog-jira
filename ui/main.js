const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

let settings = null;
let issues = [];
// auto page uses its own task list that INCLUDES Done tickets, so a PR whose
// Jira task is already Done can still be mapped (main picker stays Done-free)
let autoIssues = [];
let autoSuggestions = [];
let loggedKeys = new Set();
let timerInterval = null;
let targetHours = 8;
// lunch window (minutes-of-day) to pause the live timer readout; synced in loadSettings
let lunchEnabled = true;
let lunchStartMin = 12 * 60;
let lunchEndMin = 13 * 60;

/** minutes-of-day for a "HH:MM" string, or null if unparseable */
function hhmmToMin(v) {
  const m = /^(\d{1,2}):(\d{2})$/.exec((v || "").trim());
  if (!m) return null;
  return +m[1] * 60 + +m[2];
}

/** true if the current local time is inside the configured lunch window */
function inLunchNow() {
  if (!lunchEnabled || lunchEndMin <= lunchStartMin) return false;
  const now = new Date();
  const cur = now.getHours() * 60 + now.getMinutes();
  return cur >= lunchStartMin && cur < lunchEndMin;
}

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

/* ---- daily streak "stack" pixel-medal badge ---- */
const sparkSvg = (c) =>
  `<g fill="${c}"><rect x="18" y="16" width="8" height="4"/><rect x="14" y="20" width="16" height="4"/><rect x="18" y="24" width="8" height="4"/></g>`;

// highest tier whose `min` <= current streak wins
const STACK_TIERS = [
  { min: 30, cls: "tier-accent", glow: "stk-glow-accent",
    svg: `<g fill="#9dc4ff"><rect x="14" y="6" width="16" height="4"/><rect x="10" y="10" width="24" height="4"/></g><g fill="#6aa4ff"><rect x="10" y="14" width="24" height="4"/><rect x="12" y="18" width="20" height="4"/><rect x="14" y="22" width="16" height="4"/><rect x="16" y="26" width="12" height="4"/><rect x="18" y="30" width="8" height="4"/><rect x="20" y="34" width="4" height="4"/></g><rect x="16" y="10" width="6" height="4" fill="#eaf3ff"/>` },
  { min: 14, cls: "tier-warn", glow: "stk-glow-warn",
    svg: `<g fill="#ffcf6a"><rect x="8" y="14" width="4" height="10"/><rect x="20" y="14" width="4" height="10"/><rect x="32" y="14" width="4" height="10"/><rect x="12" y="10" width="4" height="14"/><rect x="28" y="10" width="4" height="14"/><rect x="8" y="24" width="28" height="4"/><rect x="8" y="28" width="28" height="4"/></g><rect x="9" y="14" width="4" height="4" fill="#ff7a8a"/><rect x="30" y="14" width="4" height="4" fill="#6aa4ff"/>` },
  { min: 7, cls: "tier-good", glow: "stk-glow-good",
    svg: `<g fill="#5ad6a0"><rect x="18" y="4" width="8" height="4"/><rect x="14" y="8" width="16" height="4"/><rect x="10" y="12" width="24" height="4"/><rect x="14" y="16" width="16" height="4"/><rect x="18" y="20" width="8" height="4"/></g><g fill="#2f8a63"><rect x="16" y="24" width="4" height="12"/><rect x="24" y="24" width="4" height="12"/><rect x="14" y="36" width="16" height="4"/></g><rect x="18" y="12" width="8" height="4" fill="#eafff5"/>` },
  { min: 5, cls: "tier-accent", glow: "stk-glow-accent",
    svg: `<g fill="#c4c7d2"><rect x="10" y="6" width="24" height="4"/><rect x="10" y="10" width="24" height="4"/><rect x="10" y="14" width="24" height="4"/><rect x="10" y="18" width="24" height="4"/><rect x="12" y="22" width="20" height="4"/><rect x="16" y="26" width="12" height="4"/><rect x="20" y="30" width="4" height="4"/></g><rect x="18" y="12" width="8" height="6" fill="#fff"/>` },
  { min: 3, cls: "tier-warn", glow: "stk-glow-warn",
    svg: `<g fill="#ffcf6a"><rect x="18" y="6" width="8" height="4"/><rect x="18" y="10" width="8" height="4"/><rect x="6" y="18" width="32" height="4"/><rect x="10" y="22" width="24" height="4"/><rect x="14" y="26" width="6" height="4"/><rect x="24" y="26" width="6" height="4"/><rect x="10" y="30" width="8" height="4"/><rect x="26" y="30" width="8" height="4"/></g>` },
  { min: 1, cls: "tier-good", glow: "stk-glow-good", svg: sparkSvg("#5ad6a0") },
  { min: 0, cls: "tier-dim", glow: "", svg: sparkSvg("#6b7793") },
];

function renderStack(status) {
  const el = $("stack-badge");
  if (!el) return;
  if (!status || !status.enabled) {
    el.hidden = true;
    return;
  }
  const n = status.current | 0;
  const t = STACK_TIERS.find((x) => n >= x.min) || STACK_TIERS[STACK_TIERS.length - 1];
  const glowAttr = t.glow ? ` class="${t.glow}"` : "";
  el.className = "stack-badge " + t.cls;
  el.innerHTML =
    `<svg width="20" height="20" viewBox="0 0 44 44"${glowAttr} aria-hidden="true">${t.svg}</svg>` +
    `<span class="n">${n}</span><span class="u">day streak</span>`;
  el.title = status.best ? `best ${status.best} วัน` : "";
  el.hidden = false;
}

async function refreshStack() {
  try {
    renderStack(await invoke("get_stack"));
  } catch (e) {
    /* ignore */
  }
}

/* ================= issues ================= */

function issueLabel(i) {
  return `${i.key} — ${i.summary}`;
}

function fillIssueSelect(sel, selectedKey, list = issues) {
  sel.innerHTML = '<option value="">— เลือก task —</option>';
  for (const i of list) {
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
    let prefilled = false;
    for (const b of branches.slice(0, 6)) {
      if (!b.issue_key) continue;
      const iss = issueByKey(b.issue_key);
      const chip = document.createElement("button");
      chip.className = "chip" + (b.active ? " chip-active" : "");
      // IDE badge (🖥 on the focused window); tail shows the sprint summary when
      // the branch maps to a current task, else the repo/branch it came from
      const badge = b.ide
        ? `<span class="chip-ide">${b.active ? "🖥 " : ""}${esc(b.ide)}</span> `
        : "";
      const tail = iss ? esc(iss.summary) : `${esc(b.repo)} (${esc(b.branch)})`;
      chip.innerHTML = `${badge}<b>${esc(b.issue_key)}</b> <span class="dim">· ${tail}</span>`;
      chip.title =
        `${b.path}\nbranch: ${b.branch}` +
        (iss ? `\n${iss.summary}` : "\n(ไม่อยู่ใน sprint ปัจจุบัน)");
      chip.onclick = () => {
        setIssue(b.issue_key);
        if (!issueByKey(b.issue_key)) {
          setMsg("log-msg", `${b.issue_key} ไม่อยู่ใน sprint ปัจจุบัน — ยังลงเวลาได้ถ้า key ถูกต้อง`, true);
        }
      };
      box.appendChild(chip);
      // one-shot: preselect the task the IDE is focused on, but only when the
      // user hasn't picked anything yet — never override a manual choice
      if (b.active && !prefilled && !currentIssueKey && !$("issue-search").value.trim()) {
        setIssue(b.issue_key);
        prefilled = true;
      }
    }
  } catch { /* no IDE state / no roots configured */ }
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
      // freeze the readout during lunch — the backend excludes it too
      if (inLunchNow()) {
        disp.title = (st.issue_key || "") + " (พักเที่ยง)";
        return;
      }
      disp.title = st.issue_key || "";
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
    // refresh the rest of the panel (a task moved to Done drops off the list)
    refreshToday(true);
    loadIssues(true);
    loadBranches();
  } catch (e) {
    setMsg("log-msg", String(e), false);
  } finally {
    btn.disabled = false;
  }
}

/* ================= create task ================= */
let taskMetaLoaded = false;
async function loadTaskFormMeta(force) {
  if (taskMetaLoaded && !force) return;
  const typeSel = $("create-type");
  const sprintSel = $("create-sprint");
  typeSel.innerHTML = `<option value="">กำลังโหลด…</option>`;
  sprintSel.innerHTML = `<option value="">กำลังโหลด…</option>`;
  try {
    const meta = await invoke("get_task_form_meta");
    typeSel.innerHTML = "";
    (meta.work_types || []).forEach((t) => {
      const o = document.createElement("option");
      o.value = t.id || "";
      o.dataset.name = t.name;
      o.textContent = t.name;
      typeSel.appendChild(o);
    });
    sprintSel.innerHTML = "";
    (meta.sprints || []).forEach((sp) => {
      const o = document.createElement("option");
      o.value = String(sp.id);
      o.textContent = sp.state === "active" ? `${sp.name} (active)` : sp.name;
      sprintSel.appendChild(o);
    });
    if (!meta.sprints || !meta.sprints.length) {
      sprintSel.innerHTML = `<option value="">— ไม่พบ sprint —</option>`;
    } else if (meta.default_sprint_id != null) {
      sprintSel.value = String(meta.default_sprint_id);
    }
    setMsg("create-msg", "", true);
    taskMetaLoaded = true;
  } catch (e) {
    typeSel.innerHTML = `<option value="">Task</option><option value="">Bug</option><option value="">Support</option>`;
    sprintSel.innerHTML = `<option value="">—</option>`;
    setMsg("create-msg", String(e), false);
  }
}

async function submitCreateTask() {
  const summary = $("create-summary").value.trim();
  if (!summary) return setMsg("create-msg", "กรอกหัวข้อก่อน", false);
  const typeOpt = $("create-type").selectedOptions[0];
  const sprintVal = $("create-sprint").value;
  const btn = $("btn-create-submit");
  btn.disabled = true;
  try {
    const key = await invoke("create_issue", {
      issueTypeId: typeOpt ? typeOpt.value : "",
      issueTypeName: typeOpt ? typeOpt.dataset.name || typeOpt.textContent : "",
      summary,
      description: $("create-desc").value,
      sprintId: sprintVal ? Number(sprintVal) : null,
    });
    setMsg("create-msg", `สร้าง ${key} แล้ว ✓`, true);
    stopMic();
    $("create-summary").value = "";
    $("create-desc").value = "";
    // the new task belongs in the current sprint — refresh the picker
    loadIssues(true);
    setTimeout(closeOverlays, 1000);
  } catch (e) {
    setMsg("create-msg", String(e), false);
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
      (isGh ? `<span class="src-badge pr-link" data-url="${esc(s.url)}" title="เปิด PR ในเบราว์เซอร์">PR</span>` : "");
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
    // auto page maps against the Done-inclusive list; fall back to the main
    // sprint list if it hasn't loaded yet
    fillIssueSelect(
      div.querySelector("select"),
      s.issue_key || "",
      autoIssues.length ? autoIssues : issues
    );
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

let autoFetching = false;
async function fetchAuto() {
  // guard against overlapping fetches (double-click "โหลดใหม่", or opening the
  // panel while a fetch is still in flight) — those would push the same items
  // twice into autoSuggestions and render duplicates
  if (autoFetching) return;
  autoFetching = true;
  $("auto-list").innerHTML = '<div class="hint">กำลังโหลด…</div>';
  // build into a local list and assign once at the end, so a partially-filled
  // autoSuggestions is never visible mid-fetch
  const list = [];
  let err = "";
  try {
    // Done-inclusive task list for mapping PRs (esp. PRs on already-Done tickets)
    try {
      autoIssues = (await invoke("get_sprint_issues_all")).issues || [];
    } catch {
      autoIssues = [];
    }
    // Google Calendar events
    try {
      const cal = await invoke("get_auto_suggestions");
      cal.forEach((c) => list.push({ ...c, source: "calendar" }));
    } catch (e) {
      err = String(e);
    }
    // GitHub PRs reviewed today — normalize into the same auto-suggestion shape
    // so renderAutoList()/confirmAuto() handle them without special-casing
    try {
      const prs = await invoke("get_reviewed_prs");
      prs.forEach((pr) =>
        list.push({
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
            duration_secs: 300, // PR review default = 5 นาที
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
    // safety net: drop any items sharing a dedupe key (same PR / same event)
    const seen = new Set();
    autoSuggestions = list.filter((s) => {
      const k = logKeyOf(s);
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
    // which of these were already logged today? (survives re-fetch/restart)
    try {
      loggedKeys = new Set(await invoke("get_logged_keys"));
    } catch {
      loggedKeys = new Set();
    }
    renderAutoList();
    if (err) setMsg("auto-msg", err, false);
  } finally {
    autoFetching = false;
  }
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
  $("s-stack-enabled").checked = s.stack_enabled !== false;
  $("s-stack-threshold").value = s.stack_threshold_hours ?? 6;
  $("s-remind-every").value = s.remind_every_minutes;
  $("s-remind-before").value = s.remind_before_end_minutes;
  $("s-eod").value = s.end_of_day;
  $("s-lunch-enabled").checked = s.lunch_enabled !== false;
  $("s-lunch-start").value = s.lunch_start || "12:00";
  $("s-lunch-end").value = s.lunch_end || "13:00";
  $("s-refresh").value = s.refresh_interval_minutes;
  $("s-hotkey").value = s.hotkey;
  $("s-create-hotkey").value = s.create_hotkey || "";
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
  lunchEnabled = settings.lunch_enabled !== false;
  lunchStartMin = hhmmToMin(settings.lunch_start) ?? 12 * 60;
  lunchEndMin = hhmmToMin(settings.lunch_end) ?? 13 * 60;
  fillSettingsForm(settings);
}

async function saveSettings() {
  const ns = {
    jira_base_url: $("s-jira-url").value.trim(),
    jira_email: $("s-jira-email").value.trim(),
    jira_api_token: $("s-jira-token").value.trim(),
    jira_project_key: $("s-jira-project").value.trim() || "MDT",
    work_hours_per_day: +$("s-hours").value || 8,
    stack_enabled: $("s-stack-enabled").checked,
    stack_threshold_hours: +$("s-stack-threshold").value || 6,
    remind_every_minutes: Math.max(0, +$("s-remind-every").value || 0),
    remind_before_end_minutes: Math.max(0, +$("s-remind-before").value || 0),
    end_of_day: $("s-eod").value.trim() || "18:00",
    lunch_enabled: $("s-lunch-enabled").checked,
    lunch_start: $("s-lunch-start").value.trim() || "12:00",
    lunch_end: $("s-lunch-end").value.trim() || "13:00",
    refresh_interval_minutes: Math.max(1, +$("s-refresh").value || 15),
    hotkey: $("s-hotkey").value.trim(),
    create_hotkey: $("s-create-hotkey").value.trim(),
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
    refreshStack();
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
  if (name === "createtask") loadTaskFormMeta();
}
function closeOverlays() {
  if (typeof stopMic === "function") stopMic();
  document.querySelectorAll(".overlay").forEach((el) => (el.hidden = true));
}
function anyOverlayOpen() {
  return [...document.querySelectorAll(".overlay")].some((el) => !el.hidden);
}
$("btn-open-create").onclick = () => openOverlay("createtask");
$("btn-open-auto").onclick = () => openOverlay("auto");
$("btn-open-settings").onclick = () => openOverlay("settings");
$("btn-auto-back").onclick = closeOverlays;
$("btn-settings-back").onclick = closeOverlays;
$("btn-create-back").onclick = closeOverlays;
$("btn-create-submit").onclick = submitCreateTask;

/* voice dictation (Thai) for the task-description field */
const SpeechRec = window.SpeechRecognition || window.webkitSpeechRecognition;
let recognition = null;
let micOn = false; // user intends to keep listening
let micBase = ""; // textarea value captured when dictation started
let micFinal = ""; // finalized transcript accumulated this session

function renderMic(on) {
  const btn = $("btn-mic");
  btn.classList.toggle("recording", on);
  btn.title = on
    ? "กำลังฟัง… คลิก หรือปล่อย Alt เพื่อหยุด"
    : "สั่งพิมพ์ด้วยเสียง (ภาษาไทย) — คลิก หรือกด Alt ค้าง";
}
function ensureRecognition() {
  if (recognition) return recognition;
  if (!SpeechRec) return null;
  const rec = new SpeechRec();
  rec.lang = "th-TH";
  rec.continuous = true;
  rec.interimResults = true;
  rec.onresult = (e) => {
    let interim = "";
    for (let i = e.resultIndex; i < e.results.length; i++) {
      const txt = e.results[i][0].transcript;
      if (e.results[i].isFinal) micFinal += txt;
      else interim += txt;
    }
    const desc = $("create-desc");
    desc.value = micBase + micFinal + interim;
    desc.dispatchEvent(new Event("input"));
  };
  rec.onend = () => {
    // the service auto-stops on silence — keep going if the user hasn't toggled off
    if (micOn) { try { rec.start(); } catch (_) {} }
    else renderMic(false);
  };
  rec.onerror = (e) => {
    if (e.error === "no-speech" || e.error === "aborted") return; // transient — onend will restart
    micOn = false;
    renderMic(false);
    const map = {
      "not-allowed": "ไม่ได้รับสิทธิ์ใช้ไมโครโฟน",
      "service-not-allowed": "ระบบรู้จำเสียงไม่พร้อมใช้งาน",
      "audio-capture": "ไม่พบไมโครโฟน",
      "network": "เชื่อมต่ออินเทอร์เน็ตไม่ได้ (ต้องต่อเน็ตเพื่อแปลงเสียง)",
    };
    setMsg("create-msg", map[e.error] || ("สั่งพิมพ์ด้วยเสียงล้มเหลว: " + e.error), false);
  };
  recognition = rec;
  return rec;
}
function startMic() {
  const rec = ensureRecognition();
  if (!rec) return;
  micBase = $("create-desc").value;
  if (micBase && !/\s$/.test(micBase)) micBase += " ";
  micFinal = "";
  micOn = true;
  try { rec.start(); } catch (_) {}
  renderMic(true);
}
function stopMic() {
  micOn = false;
  if (recognition) { try { recognition.stop(); } catch (_) {} }
  renderMic(false);
}
if (!SpeechRec) {
  const b = $("btn-mic");
  b.disabled = true;
  b.title = "เบราว์เซอร์ไม่รองรับการสั่งพิมพ์ด้วยเสียง";
} else {
  $("btn-mic").onclick = () => (micOn ? stopMic() : startMic());

  /* push-to-talk: hold Alt (while Create Task is open + app focused) to dictate */
  let altPtt = false;
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Alt" || e.repeat) return;
    if (e.ctrlKey || e.metaKey || e.shiftKey) return; // plain Alt only (avoid Ctrl+Alt+K clash)
    if ($("overlay-createtask").hidden) return; // only on the Create Task page
    if (micOn) return; // already listening (e.g. toggled via the button) — leave it
    e.preventDefault();
    altPtt = true;
    startMic();
  });
  window.addEventListener("keyup", (e) => {
    if (e.key !== "Alt" || !altPtt) return;
    e.preventDefault();
    altPtt = false;
    stopMic();
  });
  // Alt+Tab / losing focus won't fire keyup — stop so the mic isn't stuck on
  window.addEventListener("blur", () => {
    if (altPtt) { altPtt = false; stopMic(); }
  });
}

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
$("btn-refresh").onclick = () => { refreshToday(true); loadIssues(true); loadBranches(); taskMetaLoaded = false; };
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
/* click a PR badge → open that pull request in the default browser */
$("auto-list").addEventListener("click", (e) => {
  const badge = e.target.closest(".pr-link");
  if (!badge) return;
  const url = badge.dataset.url;
  if (url) invoke("open_url", { url }).catch(() => {});
});
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

/* ---- autostart (applies immediately, not tied to the save button) ---- */

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

/* resize grip: guard against blur auto-hide (same as move-drag), then let the
   OS run the resize loop. macOS may not support startResizeDragging — fall
   back to a manual pointer-driven setSize loop. */
document.getElementById("resize-grip").addEventListener("mousedown", async (e) => {
  if (e.button !== 0) return;
  e.preventDefault();
  invoke("begin_drag"); // cleared by mouseup / Focused(true), same as move-drag
  const w = window.__TAURI__.window.getCurrentWindow();
  try {
    await w.startResizeDragging("SouthEast");
  } catch {
    const start = await w.innerSize(); // PhysicalSize
    const sx = e.screenX, sy = e.screenY, dpr = await w.scaleFactor();
    const move = (ev) => {
      w.setSize(new window.__TAURI__.dpi.PhysicalSize(
        Math.max(1, start.width + Math.round((ev.screenX - sx) * dpr)),
        Math.max(1, start.height + Math.round((ev.screenY - sy) * dpr))
      )).catch(() => {});
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      invoke("end_drag");
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }
});

/* refresh data every time the panel opens */
listen("panel-shown", () => {
  refreshToday(false);
  refreshStack();
  loadIssues(false);
  loadBranches();
  invoke("timer_status").then(renderTimer).catch(() => {});
});

listen("total-updated", (ev) => renderToday(ev.payload));
listen("stack-updated", (ev) => renderStack(ev.payload));

/* Ctrl+Alt+K global hotkey — backend shows the window and asks us to open the
   Create-Task form. Focus the summary field so the user can type right away. */
listen("open-create", () => {
  openOverlay("createtask");
  setTimeout(() => $("create-summary").focus(), 60);
});

/* end of work day: backend already showed + pinned the panel. Stop the timer,
   keeping the elapsed prefilled so the user can log it. */
listen("work-ended", async () => {
  try {
    const st = await invoke("timer_status");
    if (st.running) {
      const stopped = await invoke("timer_stop");
      renderTimer({ running: false });
      setIssue(stopped.issue_key || "");
      $("time-input").value = fmtHM(Math.max(60, stopped.elapsed_secs));
    }
    setMsg("log-msg", "หมดเวลาทำงานแล้ว — กดบันทึกเพื่อลงเวลาที่จับไว้", true);
    invoke("get_pinned").then(renderPin).catch(() => {});
  } catch (e) {
    /* ignore */
  }
});

/* ================= init ================= */

(async function init() {
  await loadSettings();
  renderToday(null);
  refreshToday(false);
  refreshStack();
  loadIssues(false);
  loadBranches();
  loadConnStatus();
  loadProjects();
  loadGeneralInfo();
  invoke("timer_status").then(renderTimer).catch(() => {});
})();
