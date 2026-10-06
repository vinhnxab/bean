#![allow(clippy::panic)]
//! Chặn hồi quy kiến trúc cho `bean-security` (đừng để God module quay lại).
//!
//! Test này **phải** panic với thông báo rõ ràng khi hạn mức bị phá — nơi duy
//! nhất trong crate được phép dùng `panic!` ngoài phần test thường.
//!
//! Bối cảnh: `web.rs` từng là 967 dòng gộp cả `web_fetch` lẫn `web_search` +
//! test; đã tách (2026-10-05) thành `web/{fetch,search,tests}.rs`. `sandbox` (814)
//! và `ssrf` (645) là nợ riêng, chưa thuộc phạm vi test này.

use std::path::Path;

/// File module của tool web — thứ từng phình nhất sau khi gộp hai tool.
const WEB_FILE: &str = "src/web.rs";

/// Trần cho `web.rs` (chỉ còn header + consts + mod/re-export). Thật tế ~46 dòng.
const WEB_LINE_BUDGET: usize = 100;

/// Số module tối thiểu dưới `src/web/`.
const MIN_WEB_MODULES: usize = 3;

/// Trần cho **mỗi** file module trong `src/web/`.
const MODULE_LINE_BUDGET: usize = 600;

fn lines_of(relative: &str) -> usize {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("không đọc được `{relative}`: {error}"));
    text.lines().count()
}

#[test]
fn web_file_stays_under_line_budget() {
    let lines = lines_of(WEB_FILE);
    assert!(
        lines < WEB_LINE_BUDGET,
        "`{WEB_FILE}` đã {lines} dòng (trần {WEB_LINE_BUDGET}). \
         Giữ ở đây: imports, consts, mod/re-export — chia việc vào `web/fetch` + `web/search`."
    );
}

#[test]
fn web_keeps_its_extracted_modules() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/web");
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("không đọc được `src/web`: {error}"));
    let modules = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .count();
    assert!(
        modules >= MIN_WEB_MODULES,
        "`src/web/` chỉ còn {modules} module (tối thiểu {MIN_WEB_MODULES}). \
         Đừng gộp fetch/search/tests ngược lại vào `web.rs`."
    );
}

#[test]
fn web_modules_stay_under_line_budget() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/web");
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("không đọc được `src/web`: {error}"));
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("không đọc được `{}`: {error}", path.display()));
            let lines = text.lines().count();
            assert!(
                lines < MODULE_LINE_BUDGET,
                "`{}` đã {lines} dòng (trần {MODULE_LINE_BUDGET}). \
                 Tách theo trách nhiệm nghiệp vụ thay vì phình thêm.",
                path.display()
            );
        }
    }
}
