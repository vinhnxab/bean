//! # BeanAgent-memory
//!
//! Bộ nhớ bền vững: SQLite + FTS5, an toàn khi cắt lịch sử, tool bộ nhớ (agents.md mục 8).
//!
//! * [`SqliteStore`] — **một** connection duy nhất nằm ở worker thread riêng; `rusqlite` là
//!   API blocking nên không bao giờ được gọi trực tiếp trong async runtime (mục 22.8).
//! * [`MemoryStore`] — bản in-memory dùng cho test vòng lặp (giữ nguyên từ M3).
//! * [`safe_cut`] — bất biến số một của tầng này: **không bao giờ** giữ một `tool` result mà
//!   assistant gọi ra nó đã bị loại bỏ (mục 8.3, 22.1).
//! * [`memory_tool`] — `memory_save`/`memory_search` (FTS5/BM25, chuẩn hoá điểm theo nguồn).
//!
//! FTS5 có sẵn trong `libsqlite3-sys` ở chế độ `bundled` (cờ `-DSQLITE_ENABLE_FTS5` trong
//! `build.rs`) nên không cần biến môi trường đặc biệt khi build.
//! Context builder theo mục 8.2 nằm ở `beanagent_core::context` vì nó cần `WorkspaceFs` để
//! đọc `MEMORY.md`/`USER.md` qua path jail.
#![forbid(unsafe_code)]

pub mod memory_tool;
pub mod safe_cut;
pub mod store;

pub use memory_tool::memory_tools;
pub use safe_cut::{
    check_no_orphan_result, extend_start_backwards, find_compaction_start, find_safe_start,
};
pub use store::{
    MemoryRecord, MemorySearchHit, MemorySource, MemoryStore, MessageRecord, OutboxEntry,
    SessionInfo, SessionSummary, SqliteStore, Store, StoreError, StoredMessage, WebSessionInfo,
};
