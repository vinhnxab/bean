//! Lõi chung của truy vấn Bean World (`work_projects`, `work_tasks`,
//! `work_notes`) — bảng điều khiển công việc dùng chung giữa người thật và agent.
//!
//! Các truy vấn nằm ở module theo **nhóm nghiệp vụ** (`q_bw_project`,
//! `q_bw_task`, `q_bw_note`) vì trần 300 dòng/query file của test kiến trúc.
//! Module này giữ phần dùng chung (trần ký tự, validator, helper ID) và là
//! **facade** re-export truy vấn — `worker.rs` import qua `q_bw::` không đổi.
//!
//! Validator + trần ký tự ở đây là **một nguồn chân lý**: tool (`work_tool.rs`),
//! REST (`rest_bw.rs`) và cả hai store dùng lại, không tự viết luật riêng.

use crate::store::types::StoreError;

/// Trần ký tự của tên project (theo ký tự Unicode, không phải byte).
pub const PROJECT_NAME_MAX_CHARS: usize = 120;
/// Trần ký tự của tiêu đề task.
pub const TASK_TITLE_MAX_CHARS: usize = 200;
/// Trần ký tự của notes/ghi chú.
pub const NOTES_MAX_CHARS: usize = 4_000;

/// Trạng thái hợp lệ của task — kiểm tra ở **một** chỗ để tool, REST và store
/// cùng một nguồn chân lý (cùng kỹ thuật với `validate_steps` của plan).
pub const TASK_STATUSES: [&str; 4] = ["open", "in_progress", "blocked", "done"];
/// Độ ưu tiên hợp lệ.
pub const TASK_PRIORITIES: [&str; 3] = ["low", "normal", "high"];

/// Kiểm tra giá trị trạng thái trước khi ghi xuống đĩa.
pub fn validate_status(status: &str) -> Result<(), StoreError> {
    if TASK_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(StoreError::Internal(format!(
        "trạng thái `{status}` không hợp lệ — chỉ nhận: {}",
        TASK_STATUSES.join(", ")
    )))
}

/// Kiểm tra giá trị priority trước khi ghi xuống đĩa.
pub fn validate_priority(priority: &str) -> Result<(), StoreError> {
    if TASK_PRIORITIES.contains(&priority) {
        return Ok(());
    }
    Err(StoreError::Internal(format!(
        "priority `{priority}` không hợp lệ — chỉ nhận: {}",
        TASK_PRIORITIES.join(", ")
    )))
}

/// ID kiểu trait (u64) về miền SQLite (i64) — rusqlite không bind `u64` trực tiếp.
pub(super) fn id_to_sql(id: u64) -> Result<i64, StoreError> {
    i64::try_from(id).map_err(|_| StoreError::Internal("id vượt giới hạn SQLite".into()))
}

// Facade: worker.rs import mọi truy vấn Bean World qua `q_bw::` — re-export giữ
// nguyên điểm gọi khi tách file theo nhóm nghiệp vụ.
pub(super) use super::q_bw_note::{add_work_note, list_work_notes};
pub(super) use super::q_bw_project::{create_project, list_projects, set_project_status};
pub(super) use super::q_bw_task::{
    create_work_task, delete_work_task, list_work_tasks, open_work_tasks, update_work_task,
};
