#![allow(clippy::panic, clippy::expect_used)]
//! Chặn hồi quy kiến trúc ở tầng web (đừng để God file quay lại).
//!
//! # Vì sao cần
//!
//! `server.rs` từng là **một** file 1897 dòng trộn bốn việc không liên quan: middleware
//! bảo mật, ~30 handler REST, vòng lặp WebSocket và dựng router. Sửa kiểm tra `Origin`
//! (mục 15.7) buộc phải mở file chứa cả `run_socket`.
//!
//! Tầng web là **bề mặt tấn công nghiêm trọng nhất** — agent có quyền chạy lệnh, nên
//! một lỗ CSRF ở đây là một lỗ thực thi từ xa. Đó là lý do việc "file bảo mật" phải
//! được biết đích danh, không nằm lẫn trong file handler.
//!
//! Đây là "guardrail", không phải đo chất lượng: nó chỉ nói "file này đã lớn quá mức dễ
//! đọc", còn `clippy`/`fmt`/`cargo test` lo phần còn lại.

use std::path::{Path, PathBuf};

/// Trần số dòng cho **từng** file dưới `src/server/`.
/// Lớn nhất sau khi tách là `ws.rs` (~468) — dưới trần này.
const FILE_LINE_BUDGET: usize = 600;

/// Số module tối thiểu trực tiếp dưới `src/server/`.
const MIN_SERVER_MODULES: usize = 10;

/// Số handler REST (`rest_*.rs`) tối thiểu — mỗi file một nhóm tài nguyên.
const MIN_REST_GROUPS: usize = 7;

/// Các module mà doc của `mod.rs` phải nhắc tới — bản đồ để người mới đọc.
const MOD_DOC_MUST_MENTION: &[&str] = &["guard.rs", "dto.rs", "routes.rs", "ws.rs", "ui_assets.rs"];

/// Thư mục gốc `src/server`.
fn server_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server")
}

/// Đếm file `.rs` trong thư mục, không đệ quy.
fn count_rs(dir: &Path) -> usize {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("không đọc được `{}`: {error}", dir.display()));
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .count()
}

#[test]
fn no_server_file_exceeds_the_line_budget() {
    let dir = server_dir();
    let entries = std::fs::read_dir(&dir).expect("đọc `src/server`");
    let mut over = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let count = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("không đọc được `{name}`: {error}"))
            .lines()
            .count();
        if count >= FILE_LINE_BUDGET {
            over.push(format!("{name} ({count} dòng)"));
        }
    }
    assert!(
        over.is_empty(),
        "file vượt trần {FILE_LINE_BUDGET} dòng: {over:?}. \
         Tách tiếp theo ranh giới nghiệp vụ hoặc bảo mật."
    );
}

#[test]
fn server_keeps_its_extracted_modules() {
    let modules = count_rs(&server_dir());
    assert!(
        modules >= MIN_SERVER_MODULES,
        "`src/server/` chỉ còn {modules} module (tối thiểu {MIN_SERVER_MODULES}). \
         Đừng gộp ngược về một file — `guard.rs` phải tách riêng khỏi handler."
    );
}

#[test]
fn rest_handlers_stay_split_by_resource() {
    let entries = std::fs::read_dir(server_dir()).expect("đọc `src/server`");
    let groups: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("rest_").then_some(name)
        })
        .collect();
    assert!(
        groups.len() >= MIN_REST_GROUPS,
        "chỉ còn {} nhóm REST ({groups:?}), tối thiểu {MIN_REST_GROUPS}.",
        groups.len()
    );
}

/// Doc của `mod.rs` phải mô tả cây module.
#[test]
fn mod_doc_documents_the_module_tree() {
    let text = std::fs::read_to_string(server_dir().join("mod.rs")).expect("đọc `mod.rs`");
    for name in MOD_DOC_MUST_MENTION {
        assert!(
            text.contains(name),
            "`mod.rs` không nhắc tới `{name}` — bản đồ module đã lỗi thời."
        );
    }
}

/// `guard.rs` phải tồn tại và giữ **cả hai** middleware.
///
/// `request_guard` (Origin/Host/CSRF) và `security_headers` (CSP) là hai lớp bảo vệ
/// khác nhau; mất một lớp là mất một lớp phòng thủ, và lỗi đó **không ai thấy** khi
/// đọc code handler.
#[test]
fn security_middleware_is_isolated_and_complete() {
    let text = std::fs::read_to_string(server_dir().join("guard.rs")).expect("đọc `guard.rs`");
    for symbol in [
        "pub async fn request_guard",
        "pub async fn security_headers",
    ] {
        assert!(
            text.contains(symbol),
            "`guard.rs` mất `{symbol}` — đây là lớp phòng thủ bảo mật (mục 15.7)."
        );
    }
}
