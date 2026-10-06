//! [`SqliteStore`] — bản cài đặt SQLite + FTS5 (M5), bền vững qua khởi động lại.
//!
//! # Vì sao là thư mục
//!
//! Trước đây phần này nằm chung với toàn bộ tầng bộ nhớ trong một file ~4900 dòng.
//! Nay tách theo **nhóm nghiệp vụ**, và điểm cắt quan trọng nhất là ranh giới
//! **sync ⇄ async**:
//!
//! ```text
//! sqlite/
//! ├─ mod.rs          SqliteStore + impl Store (đẩy lệnh qua kênh)
//! ├─ schema.rs       SCHEMA_SQL / PRAGMA_SQL
//! ├─ command.rs      enum DbCommand — 51 lệnh + kênh trả lời
//! ├─ worker.rs       worker_loop + dispatch (chạm connection)
//! ├─ migration.rs    pragma, migration, tiện ích chung
//! └─ q_*.rs          truy vấn SQL, mỗi file một nhóm nghiệp vụ
//! ```
//!
//! # Bất biến then chốt
//!
//! `rusqlite` là API **blocking** (agents.md mục 22.8), nên **không** được gọi trực
//! tiếp trong async runtime. Mọi truy cập đi qua `DbCommand` tới worker thread duy
//! nhất giữ connection. Đó là lý do `impl_store.rs` ở đây mỗi method chỉ "đóng gói lệnh
//! rồi `.await`" — phần thật nằm ở `q_*.rs`, và đó là lý do **không được** chuyển bất kỳ
//! hàm nào trong `q_*.rs` thành `async`.

mod command;
mod impl_store;
mod inner;
pub(crate) mod migration;
mod q_history_search;
mod q_mcp;
mod q_memory_outbox;
mod q_message;
mod q_session;
mod q_task;
mod q_usage_web;
pub(crate) mod schema;
mod worker;

pub use inner::SqliteStore;

// ---------------------------------------------------------------------------
// Truy vấn SQL — mỗi file một nhóm nghiệp vụ (xem `q_*.rs`)
// ---------------------------------------------------------------------------
