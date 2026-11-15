//! Test dùng **chung cho cả hai** bản cài đặt ([`MemoryStore`](super::MemoryStore) và
//! [`SqliteStore`](super::SqliteStore)).
//!
//! # Vì sao tách thành thư mục riêng
//!
//! Trước đây 579 dòng test nằm cuối `store.rs`, chồng lên các module thật. Tách ra đây
//! giữ file sản phẩm chỉ còn code chạy thật, và khiến một điểm trở nên hiển nhiên:
//!
//! # Ràng buộc bất biến
//!
//! Mọi test bên dưới chạy **trên cả hai** bản cài đặt. Đây là cách duy nhất giữ được
//! ràng buộc "hai bản phải cùng ngữ nghĩa" — nếu chỉ test `SqliteStore`, một lỗi chỉ xảy
//! ra ở `MemoryStore` (dùng cho **mọi** test vòng lặp agent) sẽ không bao giờ bị phát
//! hiện. Thêm hàm kiểm tra mới mà quên gọi trên cả hai bản là thất lạc.

// Test thì được phép `unwrap` — nguyên tắc "không panic trong code sản phẩm"
// (agents.md mục 0.8) không áp dụng cho test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use bean_types::{Message, ToolCall};
use tempfile::TempDir;

use super::{MemorySearchHit, SqliteStore};

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("bean.db")
}

fn open(dir: &TempDir) -> SqliteStore {
    SqliteStore::open(&db_path(dir)).unwrap()
}

/// Một lượt "đọc file" hoàn chỉnh: user → assistant(gọi tool) → tool result.
fn tool_round(index: usize, filler: &str) -> Vec<Message> {
    let id = format!("c{index}");
    vec![
        Message::user(format!("{filler} câu hỏi {index}")),
        Message::assistant(
            None,
            vec![ToolCall::new(
                id.clone(),
                "probe",
                serde_json::json!({ "n": index }),
            )],
        ),
        Message::tool(id, format!("{filler} kết quả {index}")),
    ]
}

fn texts(hits: &[MemorySearchHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.text.as_str()).collect()
}

// Test tách theo ranh giới nghiệp vụ (`tests/*.rs`); nhóm mới → module mới,
// đừng nhét vào file này (trần 1000 dòng được `tests/architecture.rs` đo thật).
mod fts;
mod messages;
mod parity;
mod persist;
mod session;
