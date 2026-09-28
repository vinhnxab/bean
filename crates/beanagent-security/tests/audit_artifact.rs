//! Audit log KHÔNG được chứa payload ảnh (M26, mục 15.8).
//!
//! `browser_screenshot` trả ảnh. Ghi base64 (hàng trăm KB) vào `audit.jsonl` sẽ
//! làm phình file vô hạn theo thời gian, và audit log là nơi hay bị copy đi lưu —
//! không nên mang theo payload. Trường `artifact` chỉ ghi **tham chiếu**.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![forbid(unsafe_code)]

use beanagent_security::audit::{AuditEntry, AuditLog, entry_now};
use serde_json::json;

/// 500 ký tự base64 giả — đủ để bắt được hồi quy "gửi cả payload vào audit".
const IMAGE_DATA: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn tmp_log() -> (tempfile::TempDir, AuditLog) {
    let dir = tempfile::tempdir().expect("tạo thư mục tạm");
    let log = AuditLog::open(dir.path()).expect("mở audit log");
    (dir, log)
}

/// Bản ghi có ảnh ghi **tham chiếu**, không ghi base64.
#[test]
fn artifact_records_reference_not_payload() {
    let (dir, log) = tmp_log();
    let mut entry = entry_now(
        1,
        "cli",
        "browser_screenshot",
        &json!({ "url": "http://localhost:3000" }),
    );
    entry.ok = Some(true);
    entry.decision = "allow";
    entry.decided_by = "cli:local".into();
    entry.artifact = Some(format!(
        "image:image/png:{}:{} bytes",
        "abc123",
        IMAGE_DATA.len()
    ));
    log.record(&entry).expect("ghi được");

    let content = std::fs::read_to_string(dir.path().join("audit.jsonl")).expect("đọc được file");
    assert!(
        !content.contains(IMAGE_DATA),
        "audit KHÔNG được chứa base64 của ảnh"
    );
    // Bắt được cả trường hợp cắt bớt payload (thay vì so khớp toàn bộ chuỗi).
    assert!(
        !content.contains("iVBORw0KGgo"),
        "không được gửi bất kỳ phần payload nào"
    );
    assert!(
        content.contains("image:image/png:abc123"),
        "phải có tham chiếu để tra"
    );
}

/// Bản ghi **không** có ảnh thì không sinh trường `artifact` — không phình file.
#[test]
fn plain_entries_have_no_artifact_field() {
    let (dir, log) = tmp_log();
    let entry = entry_now(1, "cli", "read_file", &json!({ "path": "a.txt" }));
    log.record(&entry).expect("ghi được");
    let content = std::fs::read_to_string(dir.path().join("audit.jsonl")).expect("đọc được");
    assert!(
        !content.contains("artifact"),
        "bản ghi thường không được sinh trường artifact: {content}"
    );
}

/// `artifact` cũng được redact secret như mọi trường khác.
#[test]
fn artifact_goes_through_secret_redaction() {
    let dir = tempfile::tempdir().expect("tạo thư mục tạm");
    let log = AuditLog::open(dir.path()).expect("mở audit log");
    // Dùng đúng hình dạng secret mà `redact_text_secrets` nhận diện (`sk-…`):
    // test phải chứng minh `artifact` đi qua bộ lọc, chứ không phải chứng minh bộ
    // lọc bắt được mọi chuỗi trông giống secret (`token=` trần không khớp mẫu).
    let mut entry = entry_now(1, "cli", "browser_screenshot", &json!({}));
    entry.artifact = Some("image:image/png:sk-abcdef0123456789:10 bytes".into());
    log.record(&entry).expect("ghi được");
    let content = std::fs::read_to_string(dir.path().join("audit.jsonl")).expect("đọc được");
    assert!(
        !content.contains("sk-abcdef0123456789"),
        "artifact cũng phải qua redact"
    );
    assert!(content.contains("[REDACTED]"), "phải thay bằng [REDACTED]");
}

/// Dựng entry mặc định thì `artifact` là `None` (không phải chuỗi rỗng).
#[test]
fn default_entry_has_no_artifact() {
    let entry: AuditEntry = entry_now(1, "cli", "x", &json!({}));
    assert!(entry.artifact.is_none());
}
