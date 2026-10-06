#![allow(clippy::panic)]
//! Chặn hồi quy kiến trúc cho `bean-tools` (đừng để God module quay lại).
//!
//! Test này **phải** panic với thông báo rõ ràng khi hạn mức bị phá — nơi duy
//! nhất trong crate được phép dùng `panic!` ngoài phần test thường.
//!
//! Bối cảnh: `mcp.rs` từng là 930 dòng gộp config/timeout, spawn/handshake,
//! runtime nhiều server, wrapper Tool và test; đã tách (2026-10-06) thành
//! `mcp/{config,connection,runtime,tool,tests}.rs`.

use std::path::Path;

/// File facade của MCP client — thứ từng phình nhất crate.
const MCP_FILE: &str = "src/mcp.rs";

/// Trần cho `mcp.rs` (chỉ còn doc + mod/re-export). Thật tế ~16 dòng.
const MCP_LINE_BUDGET: usize = 100;

/// Số module tối thiểu dưới `src/mcp/`.
const MIN_MCP_MODULES: usize = 5;

/// Trần cho **mỗi** file module trong `src/mcp/`.
const MODULE_LINE_BUDGET: usize = 600;

fn lines_of(relative: &str) -> usize {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("không đọc được `{relative}`: {error}"));
    text.lines().count()
}

#[test]
fn mcp_file_stays_under_line_budget() {
    let lines = lines_of(MCP_FILE);
    assert!(
        lines < MCP_LINE_BUDGET,
        "`{MCP_FILE}` đã {lines} dòng (trần {MCP_LINE_BUDGET}). \
         Giữ ở đây: doc + mod/re-export — chia việc vào `mcp/config` + \
         `mcp/connection` + `mcp/runtime` + `mcp/tool`."
    );
}

#[test]
fn mcp_keeps_its_extracted_modules() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mcp");
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("không đọc được `src/mcp`: {error}"));
    let modules = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .count();
    assert!(
        modules >= MIN_MCP_MODULES,
        "`src/mcp/` chỉ còn {modules} module (tối thiểu {MIN_MCP_MODULES}). \
         Đừng gộp config/connection/runtime/tool ngược lại vào `mcp.rs`."
    );
}

#[test]
fn mcp_modules_stay_under_line_budget() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mcp");
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("không đọc được `src/mcp`: {error}"));
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
