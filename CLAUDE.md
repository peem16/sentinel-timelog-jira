# TimeLog

Tauri 2 tray app (Windows/macOS) สำหรับลงเวลา Jira — frontend เป็น vanilla HTML/CSS/JS
ใน `ui/` (ไม่มี bundler, ใช้ `withGlobalTauri`), backend Rust ใน `src-tauri/src/`
ดูภาพรวมฟีเจอร์ + โครงสร้างไฟล์ใน README.md

## คำสั่ง

- `npm run dev` — รัน dev
- `npm run build` — สร้าง installer (ต้องตั้ง env `TIMELOG_*` ถ้าจะ embed OAuth credentials — ดู README)
- ไม่มี test suite — ตรวจ Rust ด้วย `rtk cargo check` ใน `src-tauri/`

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
