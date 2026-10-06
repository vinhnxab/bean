//! Interface lưu trữ lịch sử hội thoại (agents.md mục 8).
//!
//! * **M3**: bản in-memory [`MemoryStore`] — nhanh, dùng cho test vòng lặp và demo.
//! * **M5**: bản SQLite + FTS5 [`SqliteStore`] — bền vững qua khởi động lại, tìm kiếm
//!   BM25 (`memory_search`), compaction ở **ranh giới an toàn** (crate::safe_cut).
//!
//! Cả hai cùng cài đặt **một** trait [`Store`], nên `agent::run_turn` không cần biết
//! lịch sử đang nằm ở đâu.
//!
//! Bất biến (agents.md mục 8.3, 22.1): không thao tác nào được tách một cặp
//! `assistant(tool_calls)` khỏi các `tool` result của nó — API sẽ trả 400.
//!
//! # Vì sao là thư mục chứ không phải một file
//!
//! Trước đây toàn bộ tầng này nằm trong `store.rs` ~4900 dòng. Số dòng lớn **không**
//! phải do Rust: đo thật thì comment chỉ chiếm 6,7% file. Nguyên nhân là `store.rs`
//! chứa **ba bản của cùng một danh sách 51 thao tác** (khai báo trait + impl của
//! `MemoryStore` + impl của `SqliteStore`) cộng ~1100 dòng truy vấn SQL và 578 dòng
//! test.
//!
//! Hệ quả thực tế: sửa token client MCP bắt buộc phải mở file chứa cả schema FTS5,
//! worker thread và hàng đợi task — và file đó **không có guardrail nào** canh, khác
//! hẳn `router.rs` đã có test chặn hồi quy. Đó là loại rủi ro khó bảo trì nhất: không
//! phải file dài, mà là **file lớn nhất lại ít được bảo vệ nhất**.
//!
//! Cách cắt ở đây theo **ranh giới nghiệp vụ**, không chia đều số dòng:
//!
//! ```text
//! store/
//! ├─ trait_def.rs    trait Store — bề mặt ổn định mà lớp trên chỉ cần biết
//! ├─ types.rs        kiểu record + StoreError (API công khai của crate)
//! ├─ budget.rs       ngân sách token (chính sách tầng trên, phụ thuộc store)
//! ├─ shared.rs       hàm hai bản cài đặt PHẢI dùng chung (cắt lịch sử, token)
//! ├─ compaction.rs   quy trình nén lịch sử — bất biến "không cắt đôi cặp tool"
//! ├─ memory.rs       MemoryStore (test double)
//! ├─ tests.rs        test dùng chung cho cả hai bản cài đặt
//! └─ sqlite/         SqliteStore: schema, worker thread, và truy vấn theo nhóm
//! ```
//!
//! Nợ kỹ thuật còn lại: trait `Store` vẫn gộp 8 nhóm nghiệp vụ. Tách theo ISP là
//! việc riêng, cần đổi chữ ký ở ~10 crate — xem `trait_def.rs`.

// ---------------------------------------------------------------------------
// Khai báo module
// ---------------------------------------------------------------------------

mod budget;
mod compaction;
mod memory;
mod shared;
mod sqlite;
mod trait_def;
mod types;

#[cfg(test)]
mod tests;

/// Hằng số chung: ngưỡng compaction, ước lượng token, giới hạn `memory_search`.
///
/// Ở `mod.rs` chứ không phải module sở hữu từng hằng, vì chúng được **nhiều** module
/// cùng đọc (`shared.rs` đo token, `compaction.rs` áp ngưỡng, `sqlite/` cắt preview):
/// đặt ở đây thì đổi một hằng chỉ sửa một chỗ và không phải săn tìm "ai đang giữ nó".
const COMPACT_KEEP_RECENT: usize = 20;
/// Ngưỡng kích hoạt compaction: dùng > 70% ngân sách context (agents.md mục 8.3).
const COMPACT_TRIGGER_PERCENT: u64 = 70;
/// Ước lượng token khi provider không có API đếm: `chars / 4` (agents.md mục 8.2).
const CHARS_PER_TOKEN: u64 = 4;
/// Trần ký tự đưa vào prompt tóm tắt (chặn nổ context khi lịch sử quá dài).
const MAX_SUMMARY_INPUT_CHARS: usize = 48_000;
/// Số kết quả tối đa của một lần `memory_search`.
const MEMORY_SEARCH_LIMIT: usize = 20;
/// Độ dài preview của message trả về từ `memory_search`.
const MEMORY_HIT_PREVIEW_CHARS: usize = 500;

// ---------------------------------------------------------------------------
// Re-export — đây là API công khai của crate
// ---------------------------------------------------------------------------

pub use budget::{
    ensure_daily_budget, ensure_role_daily_budget, record_usage, record_usage_for_role,
};
pub use memory::MemoryStore;
pub use sqlite::SqliteStore;
pub use trait_def::Store;
pub use types::{
    McpClientInfo, MemoryRecord, MemorySearchHit, MemorySource, MessageRecord, NewScheduledTask,
    OutboxEntry, ScheduledTask, SessionInfo, SessionSummary, StoreError, StoredMessage,
    WebSessionInfo,
};
