//! Tool nhóm `files` (agents.md mục 7.3): [`read_file`], [`list_dir`], [`glob`], [`grep`]
//! (Safe), [`write_file`], [`edit_file`] (Confirm).
//!
//! Mọi tool dùng [`crate::WorkspaceFs`] và nhận đường dẫn **tương đối với workspace**.
//! Trần đọc/ghi là `spawn_blocking` vì I/O chặn (mục 22.8).

pub mod params;
pub mod tool;
