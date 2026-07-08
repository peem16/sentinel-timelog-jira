# TimeLog

โปรแกรมบันทึกเวลาทำงานลง Jira แบบ minimal อยู่ใน system tray (Windows) / menubar (macOS)
สร้างด้วย **Tauri 2 + Rust** — frontend เป็น HTML/CSS/JS ล้วน ไม่มี framework

## ฟีเจอร์

- **Tray แสดงชั่วโมง** — เห็น `6.2/8` ตลอดเวลา
  - macOS: แสดงเป็น text ข้าง menubar
  - Windows: render ตัวเลขเป็น icon (แบบเศษส่วน บน/ล่าง) + tooltip
- **ลงเวลาเร็ว** — คลิก tray หรือกด hotkey (default `Ctrl+Alt+L`) → เลือก task จาก sprint ปัจจุบัน (ช่องเลือกพิมพ์ค้นหาจาก key/ชื่อ task ได้ เลื่อนด้วยลูกศร + Enter) ใส่เวลา (`1:30`, `1.5`, `45m`, `1h30m`) + รายละเอียด
  - รายการ task มาจาก active sprint (หา sprint จากค่า Sprint field ของ issue ล่าสุด — ไม่ใช้ JQL `openSprints()` ที่เพี้ยนบน site ที่มี Sprint custom field ซ้ำ) เฉพาะงานที่ยังไม่ Done รวม subtask เรียงตามลำดับบนบอร์ด (Rank)
- **Branch suggestion** — สแกนโฟลเดอร์โปรเจกต์ที่ตั้งค่าไว้ อ่าน branch ปัจจุบันของแต่ละ repo (ที่เปิดใน Cursor/VS Code) ถ้าชื่อ branch มี `MDT-1234` จะขึ้นเป็น chip กดเลือก task ได้เลย เรียงตาม repo ที่ active ล่าสุด
- **Timer** — ปุ่ม start/stop จับเวลา task แล้ว prefill ลงฟอร์ม
- **แจ้งเตือน** — เตือนทุก N นาทีถ้ายังลงเวลาไม่ครบ + เตือนก่อนหมดวัน
- **Auto จาก Google Calendar** — ดึงนัดวันนี้ (Calendar API ถ้า login Google แล้ว, ไม่งั้นใช้ secret ICS URL — รองรับ RRULE รายวัน/รายสัปดาห์/รายเดือน/รายปี + EXDATE), map ชื่อนัด → task ใน sprint ตามกติกาที่ตั้งไว้ (เช่น `Mandrake Grooming` → task ที่ชื่อขึ้นต้น `Grooming`) แสดง suggestion ทั้งวันให้แก้/ติ๊กเลือก แล้วกดยืนยันเพื่อลงเวลาจริงทีเดียว
- **Auto จาก GitHub PR reviews** — ดึง PR ใน org ที่เรา Comment / Approve / Request changes วันนี้ (ยืนยันจาก review/comment จริงเป็นรายตัว ไม่ใช่แค่ PR ถูกอัปเดตวันนี้) แสดงเวลา "รีวิว HH:MM" เรียงล่าสุดก่อน แล้ว map เป็น task Jira จากชื่อ PR (`[MDT-1728] …` → `MDT-1728`)
- **กันลงซ้ำ** — รายการ auto (นัด/PR) ที่ลงเวลาไปแล้ววันนี้จะติดป้าย "ลงแล้ว ✓" ขีดฆ่า จาง และกดซ้ำไม่ได้ (เก็บใน `logged.json` รีเซ็ตรายวัน — จำได้แม้ปิดเปิดแอปหรือ refresh)
- **Widget ลากย้ายได้** — ลากที่ส่วนหัว/ตัวเลขสรุปเพื่อย้ายตำแหน่ง แอปจำตำแหน่งล่าสุดไว้ (เก็บใน `window.json`) เปิดครั้งถัดไปมาที่เดิม
- **ปักหมุด 📌** — กดปุ่มหมุดที่มุมขวาบนเพื่อให้ widget ค้างอยู่บนจอ ไม่ซ่อนอัตโนมัติเมื่อคลิกที่อื่น (กดซ้ำเพื่อกลับเป็นโหมด popup ปกติ)
- **UI โปร่งใส** — acrylic (Windows) / vibrancy (macOS) + backdrop blur
- **Token ปลอดภัย** — OAuth tokens เก็บใน OS keychain (Windows Credential Manager / macOS Keychain) ไม่ใช่ไฟล์ plaintext
- **Autostart** — ติ๊ก "เปิดอัตโนมัติตอน login" ได้ในแท็บตั้งค่า (หัวข้อ ทั่วไป & อัปเดต)
- **ตรวจอัปเดต** — ปุ่มตรวจ+ติดตั้งเวอร์ชันใหม่ในแท็บตั้งค่า (คน build ต้องตั้งค่า endpoint ก่อน — ดูหัวข้อ Auto-update)

## ติดตั้ง / รัน

ต้องมี Rust toolchain + Node.js

```bash
npm install
npm run dev      # รันแบบ dev
npm run build    # สร้าง installer (.msi / .dmg)
```

## ตั้งค่าครั้งแรก (แท็บ "ตั้งค่า")

### 1. ลงทะเบียน OAuth App (ทำครั้งเดียว ใช้ได้ทั้งทีม)

OAuth บังคับว่าต้องมี "app ที่ลงทะเบียนไว้" หนึ่งตัวเสมอ — ลงทะเบียน **ครั้งเดียว** (คนที่ build/แจกทำให้ทั้งทีม) callback URL ของทั้งคู่คือ `http://127.0.0.1:53817/callback`

- **Atlassian**: [developer.atlassian.com](https://developer.atlassian.com/console/myapps/) → Create → OAuth 2.0 integration
  → Permissions: Jira API → เพิ่ม scope `read:jira-work`, `write:jira-work` (offline_access ใส่ให้อัตโนมัติตอน authorize)
  → Authorization: ใส่ callback URL ข้างบน → คัดลอก Client ID + Secret
- **Google**: [console.cloud.google.com](https://console.cloud.google.com/) → สร้าง project → เปิดใช้ **Google Calendar API**
  → OAuth consent screen (ถ้าเป็น Google Workspace เลือก **Internal** จะไม่ต้อง verify และ refresh token ไม่หมดอายุ)
  → Credentials → Create OAuth client ID → type **Desktop app** → คัดลอก Client ID + Secret
- **GitHub** (ดึง PR ที่ review วันนี้): [github.com/settings/developers](https://github.com/settings/developers) → **New OAuth App**
  → Authorization callback URL = callback URL ข้างบน → คัดลอก Client ID + Secret (กด Generate a new client secret)
  → ตอน Login เลือก scope `repo`, `read:org`, `read:user`; ถ้า org เปิด SAML SSO ต้องกด **Authorize** token กับ org ด้วย

จากนั้นเลือกวิธีใส่ credentials อย่างใดอย่างหนึ่ง:

**วิธีที่ 1 (แนะนำ) — bake เข้าไปในตัวแอปตอน build** ผู้ใช้ทุกคนแค่กด Login ไม่เห็นช่องกรอกเลย
ตั้ง environment variable ก่อน `npm run build` (ค่านี้จะถูก compile เข้า binary ไม่เข้า git):

```powershell
$env:TIMELOG_ATLASSIAN_CLIENT_ID     = "xxx"
$env:TIMELOG_ATLASSIAN_CLIENT_SECRET = "xxx"
$env:TIMELOG_GOOGLE_CLIENT_ID        = "xxx"
$env:TIMELOG_GOOGLE_CLIENT_SECRET    = "xxx"
$env:TIMELOG_GITHUB_CLIENT_ID        = "xxx"
$env:TIMELOG_GITHUB_CLIENT_SECRET    = "xxx"
npm run build
```
```bash
# macOS/Linux
TIMELOG_ATLASSIAN_CLIENT_ID=xxx TIMELOG_ATLASSIAN_CLIENT_SECRET=xxx \
TIMELOG_GOOGLE_CLIENT_ID=xxx TIMELOG_GOOGLE_CLIENT_SECRET=xxx \
npm run build
```

เมื่อ build แบบนี้ แอปจะซ่อนหัวข้อ "OAuth App" และช่อง manual ทั้งหมดอัตโนมัติ — เหลือแค่ปุ่ม Login

> หมายเหตุความปลอดภัย: Google desktop client ถือว่า secret ไม่เป็นความลับอยู่แล้ว (ใช้ PKCE) ฝังได้ปกติ ส่วน Atlassian 3LO ต้องใช้ secret จริง การฝังใน binary ที่แจกในทีมภายในถือว่ารับได้ (redirect ล็อกไว้ที่ localhost) แต่ไม่ควรเอา binary นี้ไปแจกสาธารณะ

**วิธีที่ 2 — กรอกเองในแอป** เปิดหัวข้อ **OAuth App** วาง Client ID/Secret ทั้ง 4 ช่อง → กดบันทึก (เหมาะตอน dev หรือไม่อยาก build เอง)

### 2. Login

- กด **Login ด้วย Atlassian** → เบราว์เซอร์เปิด → Allow → แอปจะรู้ site + ตัวตนเอง แล้วโหลดรายชื่อ project ให้เลือกในช่อง Project
- กด **Login ด้วย Google** → Allow → auto mode จะดึงนัดจาก Calendar API โดยตรง (แม่นกว่า ICS เพราะ Google expand นัดประจำให้เอง)
- กด **Login ด้วย GitHub** + ใส่ **Org** (เช่น `wisesight`) → แท็บ Auto จะดึง PR ที่คุณ Comment / Approve / Request changes วันนี้ มา map เป็น task Jira จากชื่อ PR (เช่น `[MDT-1728] …` → `MDT-1728`) ให้ใส่เวลาแล้วกดยืนยันลงเวลา

> ไม่อยากทำ OAuth? ยังใช้แบบ manual ได้: Jira API token + secret ICS URL อยู่ในหัวข้อย่อย "แบบ manual" ของแต่ละส่วน

### 3. อื่นๆ

- **Workspace** — ใส่ path โฟลเดอร์ที่มี git repos (สแกนลึก 1 ชั้น) เพื่อให้ suggest task จากชื่อ branch
- **กติกา auto map** เช่น คำใน calendar: `Mandrake Grooming` → prefix task: `Grooming`
- **Hotkey** — default `Ctrl+Alt+L` (ปุ่ม `fn` จับไม่ได้ในระดับ OS จึงใช้ combination ปกติแทน)

การตั้งค่าเก็บที่ `%APPDATA%\com.wisesight.timelog\` (Windows) / `~/Library/Application Support/com.wisesight.timelog/` (macOS)
— `settings.json` (การตั้งค่า) ส่วน **OAuth tokens เก็บใน OS keychain** (Windows Credential Manager /
macOS Keychain, service `com.wisesight.timelog`) — ไฟล์ `connections.json` จากเวอร์ชันเก่าจะถูก
migrate เข้า keychain แล้วลบทิ้งอัตโนมัติตอนเปิดแอปครั้งแรก

## Auto-update (สำหรับคนที่ build แจกทีม)

ปุ่ม "ตรวจอัปเดต" ในแท็บตั้งค่าจะทำงานได้ต้องตั้งค่า endpoint + ลายเซ็นก่อน (ทำครั้งเดียว):

1. สร้างคู่กุญแจ: `npx tauri signer generate -w %USERPROFILE%\.tauri\timelog.key`
   (เก็บ private key ให้ดี — ทำหายแล้วผู้ใช้เดิมจะอัปเดตต่อไม่ได้)
2. เติมค่าใน section `plugins.updater` ของ `src-tauri/tauri.conf.json` (ตอนนี้เป็นค่าว่างอยู่ — ห้ามลบ section ทิ้ง ไม่งั้นแอป panic ตอนบูต):
   ```json
   "plugins": {
     "updater": {
       "endpoints": ["https://<host>/latest.json"],
       "pubkey": "<public key จากข้อ 1>"
     }
   }
   ```
   และในหัวข้อ `bundle` เพิ่ม `"createUpdaterArtifacts": true`
3. ตอน build ตั้ง env `TAURI_SIGNING_PRIVATE_KEY` (และ `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` ถ้าตั้งรหัสไว้)
4. อัปโหลด installer + ไฟล์ `.sig` + `latest.json` ขึ้น host — ใช้ GitHub Releases ได้เลย
   (endpoint = `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`)

ยังไม่ตั้งค่า → กดตรวจอัปเดตจะขึ้นว่า updater ยังไม่ได้ตั้งค่า (ส่วนอื่นของแอปทำงานปกติ)

## โครงสร้าง

```
ui/                  frontend (vanilla HTML/CSS/JS, withGlobalTauri)
src-tauri/src/
  lib.rs             setup: tray, hotkey, scheduler แจ้งเตือน, vibrancy
  commands.rs        tauri commands ทั้งหมด
  oauth.rs           OAuth 2.0 (Atlassian 3LO + Google PKCE + GitHub) ผ่าน loopback server
  jira.rs            Jira REST client (sprint issues, worklog, ชั่วโมงวันนี้)
  github.rs          GitHub REST client (PR ที่ review วันนี้ ใน org)
  tray.rs            วาดตัวเลขเป็น tray icon (bitmap font 5x7)
  gitscan.rs         สแกน branch จาก .git/HEAD
  calendar.rs        Google Calendar API + parse ICS (RRULE รายวัน/สัปดาห์/เดือน/ปี, EXDATE)
  logged.rs          กันลงซ้ำรายวัน (logged.json — dedupe key pr:<url> / cal:<summary>|<start>)
  winstate.rs        จำตำแหน่ง widget + สถานะปักหมุด (window.json)
  settings.rs        โหลด/เซฟ settings.json
  state.rs           app state
```
