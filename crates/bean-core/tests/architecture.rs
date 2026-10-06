#![allow(clippy::panic)]
//! Chặn hồi quy kiến trúc (đừng để God class quay lại).
//!
//! Test này **phải** panic với thông báo rõ ràng khi hạn mức bị phá, nên đây là
//! nơi duy nhất trong crate được phép dùng `panic!` ngoài phần test thường.
//!
//! # Vì sao cần
//!
//! Việc tách `Router` thành các module là **thủ công** — không có gì tự ngăn
//! `router.rs` phình trở lại sau vài tính năng mới. Một lời bình luận kiểu "sẽ tách
//! sau" không có tác dụng gì.
//!
//! Vì vậy test này **đo số dòng thật trong file** và **đếm module thật**, nên không
//! thể bị vô hiệu bằng lời hứa trong comment. Nâng hạn mức khi tách hợp lý — và
//! khi đó hãy sửa cả hằng số lẫn lý do.
//!
//! Đây là "guardrail", không phải đo chất lượng: nó chỉ nói "file này đã lớn quá
//! mức dễ đọc", còn `clippy`/`fmt`/`cargo test` lo phần còn lại.

use std::path::Path;

/// File lõi của điều phối — thứ dễ phình nhất trong crate.
const ROUTER_FILE: &str = "src/router.rs";

/// Trần số dòng của `router.rs`. Sau khi tách (2026-10-05) file chỉ còn **hợp đồng
/// công khai + state + helpers (~215 dòng)**; phần intake/run/status/decision/dispatch
/// đã nằm ở `router/{admit,run,status,decision,dispatch}.rs`. Chừa biên nhỏ có chủ ý
/// để thêm một phương thức mới mà không phải sửa hằng số.
const ROUTER_LINE_BUDGET: usize = 300;

/// Số module tối thiểu dưới `router/`. Mỗi module là **một trách nhiệm đã tách**;
/// gộp lại về một file là đúng hướng ngược lại.
const MIN_ROUTER_MODULES: usize = 14;

/// Đọc file và trả về số dòng (không tính dòng trống cuối cùng).
fn lines_of(relative: &str) -> usize {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("không đọc được `{relative}`: {error}"));
    text.lines().count()
}

/// Đếm file `.rs` trực tiếp dưới `src/router/`.
fn count_router_modules() -> usize {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/router");
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("không đọc được thư mục `src/router`: {error}"));
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .count()
}

/// Trần số dòng cho **mỗi** file module trực tiếp trong `subdir` (vd: `src/router`).
const MODULE_LINE_BUDGET: usize = 600;

/// Không module nào trong `src/{subdir}/` được vượt trần — kể cả file mới thêm sau này.
fn assert_modules_under_budget(subdir: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(subdir);
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("không đọc được `{}`: {error}", dir.display()));
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

#[test]
fn router_modules_stay_under_line_budget() {
    assert_modules_under_budget("router");
}

#[test]
fn agent_modules_stay_under_line_budget() {
    assert_modules_under_budget("agent");
}

#[test]
fn router_file_stays_under_line_budget() {
    let lines = lines_of(ROUTER_FILE);
    assert!(
        lines < ROUTER_LINE_BUDGET,
        "`{ROUTER_FILE}` đã {lines} dòng (trần {ROUTER_LINE_BUDGET}). \
         Tách phần mới ra module con (chỉ giữ hợp đồng, state, helpers ở đây) \
         thay vì nhét thêm vào đây."
    );
}

#[test]
fn router_keeps_its_extracted_modules() {
    let modules = count_router_modules();
    assert!(
        modules >= MIN_ROUTER_MODULES,
        "`src/router/` chỉ còn {modules} module (tối thiểu {MIN_ROUTER_MODULES}). \
         Đừng gộp các module đã tách ngược về `router.rs`."
    );
}

/// `agent.rs` từng là một hàm ~550 dòng rồi phình tiếp; nay đã tách thành
/// `agent/{run,tool_call,error}`. Giữ hạn mức để nó không quay lại.
#[test]
fn agent_file_stays_under_line_budget() {
    let lines = lines_of("src/agent.rs");
    assert!(
        lines < 400,
        "`src/agent.rs` đã {lines} dòng (trần 400). \
         Giữ ở đây: types + facade `run_turn` + helper dùng chung; \
         vòng lặp vào `agent/run`, thực thi vào `agent/tool_call`, lỗi vào `agent/error`."
    );
}
