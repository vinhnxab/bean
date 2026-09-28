//! Bộ lưu trữ lịch sử hội thoại (agents.md mục 8).
//!
//! Re-export từ `bean_memory` để giữ import cũ (`bean_core::store::…`)
//! còn hoạt động. Từ **M5**, `SqliteStore` là bản dùng thật (SQLite + FTS5),
//! `MemoryStore` chỉ còn cho test.

pub use bean_memory::memory_tools;
pub use bean_memory::store::{
    self, MemoryRecord, MemorySearchHit, MemorySource, MemoryStore, MessageRecord, OutboxEntry,
    SessionInfo, SessionSummary, SqliteStore, Store, StoreError, StoredMessage, WebSessionInfo,
};
