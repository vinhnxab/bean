//! Audit log JSONL (agents.md mục 15.8).
//!
//! Mỗi bản ghi là một dòng JSON (một JSON object trên một dòng) ghi vào
//! `<data.dir>/audit/audit.jsonl` (append-only). Gồm: thời điểm RFC3339 UTC, phiên, kênh,
//! tool, **tham số đã redact secret**, kết quả, và ai quyết định cho phép.
//!
//! Redact: mọi chuỗi nằm dưới một khoá "nhạy cảm" (`key`, `token`, `secret`, `password`,
//! `authorization`, `credential` — so theo từ khoá con, không phân biệt hoa/thường) bị thay
//! bằng `[REDACTED]`. API key không bao giờ đi vào args của tool, nhưng đây là lớp phòng
//! thủ khi model tự đặt secret vào tham số (ví dụ chép từ env vào `run_shell`).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

/// Lỗi audit log.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// Lỗi I/O (tạo thư mục, mở/ghi file).
    #[error("audit log I/O: {0}")]
    Io(#[from] std::io::Error),
    /// Lỗi serialize/lock (không thể xảy ra trong điều kiện bình thường, phòng hờ).
    #[error("audit log nội bộ: {0}")]
    Internal(String),
}

/// Một bản ghi audit — một dòng JSONL.
#[derive(Debug, Serialize)]
pub struct AuditEntry {
    /// Thời điểm ghi (RFC3339 UTC).
    pub ts: String,
    /// Phiên hội thoại.
    pub session: i64,
    /// Kênh (`cli`, `web`, `telegram`, `scheduler`...).
    pub channel: String,
    /// Tên tool hoặc loại sự kiện.
    pub tool: String,
    /// Tham số đã redact secret.
    pub args: serde_json::Value,
    /// Tool chạy thành công không (`None` = chưa chạy).
    pub ok: Option<bool>,
    /// Quyết định.
    pub decision: &'static str,
    /// Ai quyết định.
    pub decided_by: String,
    /// Tóm tắt lỗi nếu có.
    pub error: Option<String>,
}

/// File JSONL append-only.
#[derive(Debug)]
pub struct AuditLog {
    file: Mutex<File>,
    path: PathBuf,
}

impl AuditLog {
    /// Mở (và tự tạo thư mục + file) audit log trong `dir`.
    ///
    /// # Errors
    /// [`AuditError::Io`] khi không tạo được thư mục hoặc không mở được file ghi.
    pub fn open(dir: &Path) -> Result<Self, AuditError> {
        fs::create_dir_all(dir)?;
        let path = dir.join("audit.jsonl");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            file: Mutex::new(file),
            path,
        })
    }

    /// Đường dẫn file audit (hiển thị/log).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ghi một bản ghi (một dòng). Ghi là nguyên tử theo dòng: một lần `write_all`
    /// nội dung đã kết thúc bằng `\n`.
    ///
    /// # Errors
    /// [`AuditError`] khi serialize/ghi thất bại. Lỗi ghi **không** được làm hỏng vòng
    /// lặp agent — caller nên log cảnh báo và tiếp tục.
    pub fn record(&self, entry: &AuditEntry) -> Result<(), AuditError> {
        let mut line =
            serde_json::to_string(entry).map_err(|e| AuditError::Internal(e.to_string()))?;
        line.push('\n');
        let mut file = self
            .file
            .lock()
            .map_err(|_| AuditError::Internal("audit lock poisoned".to_string()))?;
        file.write_all(line.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    /// Đọc tối đa `limit` dòng gần nhất, theo thứ tự mới trước.
    #[must_use]
    pub fn read_recent(&self, before: Option<u64>, limit: usize) -> Vec<serde_json::Value> {
        let content = fs::read_to_string(&self.path).unwrap_or_default();
        let mut lines: Vec<_> = content.lines().rev().collect();
        if let Some(before) = before {
            let start = usize::try_from(before).unwrap_or(lines.len());
            lines = lines.split_off(start.min(lines.len()));
        }
        if limit > 0 {
            lines.truncate(limit);
        }
        lines
            .into_iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// Tạo bản ghi với thời điểm hiện tại (RFC3339 UTC) — tiện cho caller.
#[must_use]
pub fn entry_now(session: i64, channel: &str, tool: &str, args: &serde_json::Value) -> AuditEntry {
    AuditEntry {
        ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        session,
        channel: channel.to_string(),
        tool: tool.to_string(),
        args: redact_secrets(args),
        ok: None,
        decision: "n/a",
        decided_by: "n/a".into(),
        error: None,
    }
}

/// Khoá được coi là nhạy cảm (so **contains**, lowercase).
const SENSITIVE_KEY_PARTS: &[&str] = &[
    "key",
    "token",
    "secret",
    "password",
    "authorization",
    "credential",
];

/// Trả về bản sao của `args` với mọi giá trị chuỗi dưới khoá nhạy cảm bị thay
/// bằng `[REDACTED]`.
#[must_use]
pub fn redact_secrets(args: &serde_json::Value) -> serde_json::Value {
    let mut clone = args.clone();
    redact_in_place(&mut clone);
    clone
}

fn redact_in_place(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                let sensitive = SENSITIVE_KEY_PARTS
                    .iter()
                    .any(|p| key.to_lowercase().contains(p));
                match (sensitive, map.get_mut(&key)) {
                    (true, Some(v)) => {
                        if !v.is_null() {
                            *v = serde_json::Value::String("[REDACTED]".to_string());
                        }
                    }
                    (false, Some(v)) => redact_in_place(v),
                    (true, None) | (false, None) => {}
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_in_place(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use serde_json::json;

    fn tmp_audit() -> (tempfile::TempDir, AuditLog) {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::open(dir.path()).unwrap();
        (dir, log)
    }

    #[test]
    fn record_writes_valid_jsonl() {
        let (dir, log) = tmp_audit();
        let entry = AuditEntry {
            ts: "2026-09-22T00:00:00Z".to_string(),
            session: 1,
            channel: "cli".to_string(),
            tool: "run_shell".to_string(),
            args: json!({"command": "ls"}),
            ok: Some(true),
            decision: "allow",
            decided_by: "cli:local".into(),
            error: None,
        };
        log.record(&entry).unwrap();
        log.record(&entry).unwrap();
        let content = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed["tool"], "run_shell");
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn redacts_sensitive_keys_recursively() {
        let args = json!({
            "command": "echo $ANTHROPIC_API_KEY",
            "env": {
                "API_KEY": "sk-ant-1234567890",
                "HOME": "/home/vinh",
                "auth_token": "tok_abc",
                "nested": [{ "password": "hunter2", "user": "bob" }]
            }
        });
        let out = redact_secrets(&args);
        assert_eq!(out["command"], "echo $ANTHROPIC_API_KEY");
        assert_eq!(out["env"]["API_KEY"], "[REDACTED]");
        assert_eq!(out["env"]["HOME"], "/home/vinh");
        assert_eq!(out["env"]["auth_token"], "[REDACTED]");
        assert_eq!(out["env"]["nested"][0]["password"], "[REDACTED]");
        assert_eq!(out["env"]["nested"][0]["user"], "bob");
        // Bản gốc không đổi.
        assert_eq!(args["env"]["API_KEY"], "sk-ant-1234567890");
    }

    #[test]
    fn recorded_entry_has_redacted_args_and_no_secret_in_file() {
        let (dir, log) = tmp_audit();
        let mut entry = entry_now(7, "cli", "run_shell", &json!({ "api_key": "sk-ant-xyz" }));
        entry.ok = Some(false);
        entry.error = Some("thất bại".to_string());
        log.record(&entry).unwrap();
        let content = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(!content.contains("sk-ant-xyz"), "{content}");
        assert!(content.contains("[REDACTED]"));
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["session"], 7);
        assert_eq!(parsed["error"], "thất bại");
    }
}
