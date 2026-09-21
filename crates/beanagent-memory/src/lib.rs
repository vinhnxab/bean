//! # BeanAgent-memory
//!
//! Bộ nhớ bền vững: SQLite + FTS5, xây dựng context, compaction (agents.md mục 8).
//!
//! * **M5**: `tokio-rusqlite 0.8` với feature `bundled` (FTS5 có sẵn trong `bundled`),
//!   migration theo `user_version`, trait `Store` cho lịch sử, context builder theo ngân sách
//!   token, compaction **không bao giờ** cắt giữa cặp `assistant(tool_calls)` và `tool` result.
#![forbid(unsafe_code)]
