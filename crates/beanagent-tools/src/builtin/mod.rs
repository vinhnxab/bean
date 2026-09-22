//! Tool built-in (agents.md mục 7.3).
//!
//! * **M3** — nhóm `files`: [`read_file`], [`list_dir`], [`glob`], [`grep`] (Safe),
//!   [`write_file`], [`edit_file`] (Confirm). Đều nhận `offset`/`limit` để đọc tiếp
//!   khi kết quả dài.
//! * **M4** — nhóm `shell`: `run_shell` (Confirm, sandbox).
//! * **M7** — nhóm `web`: `web_fetch`, `web_search`.
//! * **M5/M6** — nhóm `memory`/`skills`: `memory_save`/`memory_search`, `load_skill`/`create_skill`.

pub mod files;

/// Tên nhóm tool file trong `[tools] enabled` (xem `beanagent_types::config::KNOWN_TOOL_GROUPS`).
pub const GROUP_FILES: &str = "files";

/// Tất cả tool của nhóm `files` (mục 7.3). Workspace được lấy từ `ToolCtx` lúc gọi,
/// nên các tool này không cần tham số khởi tạo.
#[must_use]
pub fn file_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        files::tool::read_file(),
        files::tool::list_dir(),
        files::tool::glob(),
        files::tool::grep(),
        files::tool::write_file(),
        files::tool::edit_file(),
    ]
}

use std::sync::Arc;

use crate::tool::Tool;
