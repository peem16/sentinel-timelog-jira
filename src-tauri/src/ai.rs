//! AI-assisted issue writing: pipe a Thai prompt into a user-configurable CLI
//! (default: Claude Code headless) and parse the rewritten summary/description
//! back. The prompt travels over stdin so Thai/multi-line text never touches
//! shell quoting.

use serde::Serialize;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;

/// AI-rewritten issue draft returned to the Create Task form.
#[derive(Debug, Serialize)]
pub struct AiDraft {
    pub summary: String,
    pub description: String,
}

/// Compose the one-shot prompt. We no longer inject a description skeleton from
/// settings — the configured `skill` owns the team's issue pattern and decides
/// the structure. The prompt hands over the raw input and pins the output
/// contract (single JSON object, plain-text description) so the answer survives
/// `parse_draft` and drops cleanly into the Create Task form.
///
/// `skill` is the skill name from settings (e.g. `mdt-task-writer`). When set we
/// prepend `/<skill>` as the first line: Claude Code resolves a leading slash as
/// a skill invocation, so the skill loads for sure instead of us merely asking
/// for it in prose. Blank `skill` drafts with no skill.
pub fn build_prompt(issue_type: &str, summary: &str, description: &str, skill: &str) -> String {
    let skill = skill.trim();
    // Leading /slash-command → deterministic skill load. Keep a matching prose
    // line too so the intent is explicit even if the CLI is swapped for one that
    // ignores slash prefixes.
    let skill_prefix = if skill.is_empty() {
        String::new()
    } else {
        format!("/{skill}\n\n")
    };
    let use_skill = if skill.is_empty() {
        "เรียบเรียงข้อมูลดิบด้านล่างเป็น issue เดียวตาม pattern ของทีม".to_string()
    } else {
        format!("ใช้ skill \"{skill}\" เรียบเรียงข้อมูลดิบด้านล่างเป็น issue เดียวตาม pattern ของทีม")
    };
    format!(
        "{skill_prefix}คุณคือผู้ช่วยเขียน Jira issue ภาษาไทยของทีม Mandrake (MDT) \
ให้{use_skill} \
(คงศัพท์เทคนิค/ชื่อระบบ/โค้ดเป็นภาษาอังกฤษ)

ประเภทงาน: {issue_type}

ข้อมูลดิบจากผู้ใช้:
หัวข้อ: {summary}
รายละเอียด: {description}

ขอบเขตงานนี้ (สำคัญ):
- นี่คือ quick-draft สำหรับฟอร์ม \"สร้าง Task\" ของ TimeLog — ทำเป็น issue เดียวเท่านั้น อย่าแตก subtask และอย่าสร้าง/ยิงขึ้น Jira เอง
- ทำงานแบบ non-interactive: ถ้าข้อมูลไม่พอ ให้เขียนเท่าที่รู้และคงหัวข้อว่างไว้ตาม pattern — อย่าถามกลับ อย่ากุข้อมูลที่ผู้ใช้ไม่ได้บอก

ข้อกำหนดผลลัพธ์ (สำคัญมาก):
- ตอบเป็น JSON object เดียวเท่านั้น ห้ามมีข้อความอื่น ห้ามใช้ code fence: {{\"summary\": \"...\", \"description\": \"...\"}}
- summary: หัวข้อสั้น 1 บรรทัด
- description: plain text ตาม pattern ของ skill (ใส่หัวข้อให้ครบตามลำดับ เว้นเนื้อหาว่างได้) ใช้ \"- \" นำหน้ารายการ, \"1. \" สำหรับขั้นตอน, เว้นบรรทัดว่างคั่นหัวข้อ ห้ามใช้ markdown อื่น (** ## ฯลฯ)"
    )
}

/// Build the platform shell that runs the user's command template. Going
/// through a shell is what lets the template stay a plain string *and* find
/// `.cmd` shims / PATH entries the way the user's terminal would.
#[cfg(target_os = "windows")]
fn shell_command(template: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new("cmd");
    c.arg("/C");
    // raw_arg: hand the template to cmd.exe unquoted — Rust's auto-quoting
    // would fight cmd's own quote rules once a template contains quotes
    c.raw_arg(template);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW, same as gitscan::run_hidden
    c
}

#[cfg(not(target_os = "windows"))]
fn shell_command(template: &str) -> std::process::Command {
    let mut c = std::process::Command::new("sh");
    // -l: login shell so PATH additions (~/.local/bin, nvm, ...) are loaded
    c.args(["-lc", template]);
    c
}

/// Run the CLI template with `prompt` on stdin and return raw stdout.
pub async fn run_cli(template: &str, prompt: &str, timeout: Duration) -> Result<String, String> {
    let template = template.trim();
    if template.is_empty() {
        return Err("ยังไม่ได้ตั้งค่าคำสั่ง AI CLI — ไปที่ ตั้งค่า → AI ช่วยเขียน Task/Bug".into());
    }

    let mut cmd = tokio::process::Command::from(shell_command(template));
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true); // timeout drops the child future → process is killed

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("เรียก shell เพื่อรันคำสั่ง AI ไม่ได้: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| format!("ส่ง prompt ให้ AI ไม่ได้: {e}"))?;
        // stdin dropped here → pipe closes → the CLI starts answering
    }

    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| "AI ใช้เวลานานเกินไป (เกิน 5 นาที) — ยกเลิกแล้ว ลองใหม่อีกครั้ง".to_string())?
        .map_err(|e| format!("อ่านผลลัพธ์จาก AI ไม่ได้: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        // claude prints its JSON envelope even on exit 1 — surface the message
        // inside it instead of the raw JSON blob
        if let Some(msg) = envelope_error(&stdout) {
            return Err(friendly_ai_error(&msg));
        }
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        // a missing CLI doesn't fail spawn() — the shell exits non-zero instead
        let combined = format!("{stderr}\n{stdout}").to_lowercase();
        if combined.contains("is not recognized")
            || combined.contains("not found")
            || combined.contains("no such file")
        {
            return Err(
                "ไม่พบคำสั่ง AI CLI ในเครื่อง — ติดตั้ง Claude Code ก่อน หรือใส่ path เต็มของคำสั่งใน ตั้งค่า → AI ช่วยเขียน Task/Bug"
                    .into(),
            );
        }
        let detail = if stderr.trim().is_empty() { &stdout } else { &stderr };
        return Err(format!(
            "AI CLI ล้มเหลว ({}): {}",
            out.status,
            truncate(detail.trim(), 300)
        ));
    }
    Ok(stdout)
}

/// Parse the CLI output into a draft. Handles the `--output-format json`
/// envelope, stray code fences, and prose around the JSON object.
pub fn parse_draft(raw: &str) -> Result<AiDraft, String> {
    let mut answer = raw.trim().to_string();

    // 1) claude --output-format json envelope: {"result":"...","is_error":false,...}
    if let Some(msg) = envelope_error(&answer) {
        return Err(friendly_ai_error(&msg));
    }
    if let Ok(v) = serde_json::from_str::<Value>(&answer) {
        if let Some(r) = v.get("result").and_then(Value::as_str) {
            answer = r.trim().to_string();
        }
        // else: the CLI printed the {"summary","description"} object directly
    }

    // 2) strip ``` fences if the model ignored the no-fence instruction
    if answer.starts_with("```") {
        answer = answer
            .trim_start_matches(|c| c != '\n')
            .trim_start_matches('\n')
            .trim_end_matches("```")
            .trim()
            .to_string();
    }

    // 3) outermost {...} — '{' and '}' are ASCII so byte indices are char-safe
    let (a, b) = match (answer.find('{'), answer.rfind('}')) {
        (Some(a), Some(b)) if b > a => (a, b),
        _ => return Err(format!("AI ไม่ได้ตอบเป็น JSON: {}", truncate(&answer, 200))),
    };
    let v: Value = serde_json::from_str(&answer[a..=b])
        .map_err(|_| format!("อ่าน JSON จาก AI ไม่ได้: {}", truncate(&answer, 200)))?;

    let summary = v["summary"].as_str().unwrap_or("").trim().to_string();
    let description = v["description"].as_str().unwrap_or("").trim().to_string();
    if summary.is_empty() {
        return Err("AI ไม่ได้ส่งหัวข้อกลับมา — ลองกดใหม่อีกครั้ง".into());
    }
    Ok(AiDraft {
        summary,
        description,
    })
}

/// If `raw` is a `--output-format json` envelope reporting an error, return
/// the human message inside it.
fn envelope_error(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw.trim()).ok()?;
    if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
        Some(
            v.get("result")
                .and_then(Value::as_str)
                .unwrap_or("ไม่ทราบสาเหตุ")
                .to_string(),
        )
    } else {
        None
    }
}

/// Map known CLI error messages to actionable Thai text.
fn friendly_ai_error(msg: &str) -> String {
    if msg.to_lowercase().contains("not logged in") {
        return "Claude Code ตอบว่า \"Not logged in\" — ถ้าคำสั่งใน ตั้งค่า → AI ช่วยเขียน มี --bare ให้เอาออก (มันข้าม credentials) หรือถ้ายังไม่เคย login ให้เปิด terminal รัน claude แล้วพิมพ์ /login".into();
    }
    format!("AI ตอบกลับเป็น error: {}", truncate(msg, 300))
}

/// Char-boundary-safe truncation (byte slicing would panic mid-Thai-codepoint).
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_json_envelope() {
        let raw = r#"{"type":"result","is_error":false,"result":"{\"summary\":\"หัวข้อ\",\"description\":\"- ข้อแรก\"}","session_id":"x"}"#;
        let d = parse_draft(raw).unwrap();
        assert_eq!(d.summary, "หัวข้อ");
        assert_eq!(d.description, "- ข้อแรก");
    }

    #[test]
    fn parses_bare_json() {
        let d = parse_draft(r#"{"summary":"Fix login","description":"รายละเอียด"}"#).unwrap();
        assert_eq!(d.summary, "Fix login");
    }

    #[test]
    fn parses_fenced_json_with_prose() {
        let raw = "นี่คือผลลัพธ์:\n```json\n{\"summary\":\"S\",\"description\":\"D\"}\n```";
        let d = parse_draft(raw).unwrap();
        assert_eq!(d.summary, "S");
        assert_eq!(d.description, "D");
    }

    #[test]
    fn surfaces_envelope_error() {
        let raw = r#"{"is_error":true,"result":"Invalid API key"}"#;
        let e = parse_draft(raw).unwrap_err();
        assert!(e.contains("Invalid API key"));
    }

    #[test]
    fn not_logged_in_gets_actionable_hint() {
        let raw = r#"{"is_error":true,"result":"Not logged in · Please run /login"}"#;
        let e = parse_draft(raw).unwrap_err();
        assert!(e.contains("/login"), "{e}");
        assert!(e.contains("--bare"), "{e}");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_draft("no json here at all").is_err());
        assert!(parse_draft(r#"{"summary":""}"#).is_err());
    }

    #[test]
    fn skill_is_forced_as_leading_slash_command() {
        let p = build_prompt("Task", "หัวข้อ", "รายละเอียด", "mdt-task-writer");
        assert!(p.starts_with("/mdt-task-writer\n\n"), "{p}");
        assert!(p.contains("ใช้ skill \"mdt-task-writer\""), "{p}");
    }

    #[test]
    fn blank_skill_drops_the_slash_prefix() {
        let p = build_prompt("Task", "หัวข้อ", "รายละเอียด", "");
        assert!(!p.starts_with('/'), "{p}");
        assert!(!p.contains("ใช้ skill \""), "{p}");
        // whitespace-only is treated as blank too
        let p2 = build_prompt("Task", "หัวข้อ", "รายละเอียด", "  ");
        assert!(!p2.starts_with('/'), "{p2}");
    }

    #[test]
    fn truncate_is_char_safe() {
        let s = "ภาษาไทยยาวๆ";
        assert_eq!(truncate(s, 4), "ภาษา…");
        assert_eq!(truncate("ab", 5), "ab");
    }
}
