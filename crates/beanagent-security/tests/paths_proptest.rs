//! Property-based test cho path jail (agents.md mục 15.1/20 — bắt buộc bằng `proptest`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use beanagent_security::CapWorkspace;
use beanagent_tools::WorkspaceFs;
use proptest::prelude::*;

/// Chuỗi đường dẫn "xấu": regex sinh `/`, `.`, chữ thường (khai sinh được `..`, `./`, `//`)
/// cộng vài case tuyệt đối tường minh.
fn evil_path() -> impl Strategy<Value = String> {
    prop_oneof![
        proptest::string::string_regex("[a-z./]{0,20}").unwrap(),
        Just("/etc/passwd".to_string()),
        Just("../../etc/passwd".to_string()),
        Just("/".to_string()),
        Just("..".to_string()),
        Just("....//....//etc".to_string()),
    ]
}

proptest! {
    /// Bất biến 1: `resolve()` hoặc lỗi, hoặc trả về đường dẫn VẪN ở trong root
    /// (không tuyệt đối, không thành phần `..`).
    #[test]
    fn prop_resolve_never_escapes(rel in evil_path()) {
        let dir = tempfile::tempdir().unwrap();
        let ws = CapWorkspace::open(dir.path().to_path_buf()).unwrap();
        if let Ok(resolved) = ws.resolve(&rel) {
            prop_assert!(!resolved.is_absolute(), "{rel} -> {resolved:?}");
            prop_assert!(
                !resolved.components().any(|c| matches!(c, std::path::Component::ParentDir)),
                "{rel} -> {resolved:?}"
            );
        }
    }

    /// Bất biến 2: workspace chỉ có một file `a.txt` chứa MARKER — `read_text()` với
    /// bất kỳ đường dẫn nào chỉ được trả **lỗi** hoặc MARKER (không bao giờ nội dung ngoài).
    #[test]
    fn prop_read_never_leaks_outside(rel in evil_path()) {
        let dir = tempfile::tempdir().unwrap();
        let ws = CapWorkspace::open(dir.path().to_path_buf()).unwrap();
        ws.write_text("a.txt", "MARKER-BEN-TRONG").unwrap();
        match ws.read_text(&rel) {
            Err(_) => {}
            Ok(text) => prop_assert!(
                text == "MARKER-BEN-TRONG",
                "đọc được nội dung lạ từ `{rel}`: {text}"
            ),
        }
    }

    /// Bất biến 3: file canary ngoài workspace không bao giờ bị thay đổi qua
    /// `write_text()` với đường dẫn tùy ý.
    #[test]
    fn prop_write_never_touches_outside(rel in evil_path()) {
        let dir = tempfile::tempdir().unwrap();
        let ws = CapWorkspace::open(dir.path().to_path_buf()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let canary = outside.path().join("canary.txt");
        std::fs::write(&canary, "NGUYEN-BAN").unwrap();
        let _ = ws.write_text(&rel, "GHI-TU-TEST");
        let now = std::fs::read_to_string(&canary).unwrap();
        prop_assert_eq!(now.as_str(), "NGUYEN-BAN");
    }
}
