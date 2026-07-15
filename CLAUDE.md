# TimeLog

Tauri 2 tray app (Windows/macOS) สำหรับลงเวลา Jira — frontend เป็น vanilla HTML/CSS/JS
ใน `ui/` (ไม่มี bundler, ใช้ `withGlobalTauri`), backend Rust ใน `src-tauri/src/`
ดูภาพรวมฟีเจอร์ + โครงสร้างไฟล์ใน README.md

## คำสั่ง

- `npm run dev` — รัน dev
- `npm run build` — สร้าง installer (ต้องตั้ง env `TIMELOG_*` ถ้าจะ embed OAuth credentials — ดู README)
- ตรวจ Rust ด้วย `rtk cargo check` ใน `src-tauri/`; unit tests (parse_draft ใน
  ai.rs + adf ใน jira.rs) รันด้วย `rtk cargo test`

## กับดักที่เจอมาแล้ว (อย่าเดินซ้ำ)

- **ห้ามใช้ JQL `openSprints()` / Agile API บน site นี้** — เคยคืนค่าว่างทั้งที่มี active
  sprint เพราะ site มี Sprint custom field ซ้ำหลายตัว วิธีที่ใช้อยู่ (jira.rs): ดึง issue
  ล่าสุดของโปรเจกต์ → หา field ที่ schema เป็น `com.pyxis.greenhopper.jira:gh-sprint`
  (บน site นี้คือ `customfield_10020`) → อ่าน sprint object เอา `state=active`
- Query task ใน sprint: `sprint = <id> AND statusCategory != Done ORDER BY Rank ASC`
  รวม subtask, กรอง epic ด้วย `hierarchyLevel <= 0` — จำนวนจึงไม่เท่าหน้าบอร์ด
  (บอร์ดซ่อน subtask ใน parent)
- OAuth client id/secret ฝังตอน build ผ่าน `build.rs` (อ่าน `TIMELOG_*` จาก env หรือ
  `.env` ที่ root ก่อน `src-tauri/`) — แก้ `.env` แล้วต้อง rebuild ค่าถึงเปลี่ยน
- GitHub PR review ใช้ 2 ขั้น: search API หา candidate (`updated:>=วันนี้`) →
  ยืนยันรายตัวจาก `/reviews` + `/comments` ว่ามี activity ของเราวันนี้จริง พร้อมเก็บ
  เวลาไว้แสดง "รีวิว HH:MM"
- AI ช่วยเขียน Task/Bug (ai.rs): shell out ตาม `ai_command` ใน settings (default
  `claude -p --strict-mcp-config --allowedTools "Read Grep Glob Bash(git:*)" --output-format json`)
  — **โครงรายละเอียด/รูปแบบ issue ให้ skill `mdt-task-writer` เป็นคนตัดสินใจ** (ไม่มี
  `ai_pattern_*` ใน settings แล้ว) `build_prompt` แค่ส่งข้อมูลดิบ + สั่งให้ใช้ skill + คุม
  output contract เป็น JSON `{summary, description}` (issue เดียว ไม่แตก subtask ไม่ยิง
  ขึ้น Jira) — skill ติดตั้งที่ `~/.claude/skills/mdt-task-writer` (symlink →
  `D:\Work\Project\mdt-skills`). `--allowedTools "Read Grep Glob Bash(git:*)"` (จำกัดเฉพาะ
  read/git ไม่มี Edit/Write/general Bash) ให้ skill อ่านโค้ดจริงเติม Reference `path:line`
  + ดึง git permalink SHA ได้ในโหมด headless โดยไม่ prompt — แลกกับช้าลง จึงตั้ง timeout
  เป็น **300s** (`commands.rs::ai_draft_issue`). `settings::load` migrate default เก่าทุกตัว
  (`LEGACY_AI_COMMANDS`: `--bare`, no-tools, read-only) → default ปัจจุบันให้อัตโนมัติ
  (ค่าที่ user แก้เองไม่แตะ) —
  **prompt ส่งทาง stdin
  เสมอ** (เลี่ยง quoting ไทย/multi-line บน cmd.exe), Windows รันผ่าน `cmd /C` +
  `raw_arg` + CREATE_NO_WINDOW, macOS ผ่าน `sh -lc`, timeout 300s + `kill_on_drop`
  — กับดัก 3 ตัว: (1) **ห้ามใช้ `--bare`** — มันข้าม credentials ของ CLI ทำให้ทุก
  call ตอบ "Not logged in" (settings::load มี migration แก้ค่าเก่าให้แล้ว) ใช้
  `--strict-mcp-config` แทนถ้าอยากให้ start เร็ว (2) CLI หายไม่ทำให้ spawn() fail
  (shell รันได้เสมอ) ต้อง sniff stderr เอา ("is not recognized"/"not found")
  (3) `adf()` ใน jira.rs ห้าม emit text node ว่าง (Jira ตอบ 400) — มี unit tests
  คุมทั้ง parse_draft และ adf (`rtk cargo test`)

## ไฟล์ config ฝั่งผู้ใช้

อยู่ที่ `%APPDATA%\com.wisesight.timelog\` (Windows) / `~/Library/Application Support/com.wisesight.timelog/` (macOS):

- `settings.json` — การตั้งค่าทั้งหมด (รวม OAuth app credentials แบบกรอกเอง)
- **OAuth tokens อยู่ใน OS keychain** (ไม่ใช่ไฟล์) — service `com.wisesight.timelog`
  จัดการใน oauth.rs (`kr_read`/`kr_write`) มี 2 กับดักบน Windows Credential Manager:
  (1) limit 2560 bytes เก็บเป็น UTF-16 = เหลือจริง ~1280 ตัวอักษร แต่ refresh token
  Atlassian ยาว ~2000 ตัวอักษร จึง**หั่นค่าเป็นชิ้นละ ≤1000 ตัวอักษร** ต่อหลาย entry
  (`atlassian`, `atlassian#2`, …) แล้วต่อกลับตอนอ่าน (2) ตอน persist ตัด access token
  ทิ้งถ้ามี refresh token (เปิดแอปครั้งถัดไป refresh ใหม่เอง) — ไฟล์ `connections.json`
  เดิมถูก migrate เข้า keychain แล้วลบอัตโนมัติใน `load()`
- `logged.json` — dedupe รายการ auto ที่ลงแล้ววันนี้ (key: `pr:<url>` / `cal:<summary>|<start>` — สร้างฝั่ง frontend ใน main.js `logKeyOf`)
- `window.json` — ตำแหน่ง widget + สถานะปักหมุด
