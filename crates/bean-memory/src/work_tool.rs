//! Tools Bean World — bảng điều khiển công việc `Project → Task → Note` dùng chung
//! giữa người thật (REST/UI) và agent (tools ở đây): **một** nguồn sự thật nằm ở store.
//!
//! # Ranh giới bắt buộc (viết thẳng vào description cho model)
//!
//! Model dễ nhầm bốn nơi ghi "việc cần làm", nên description của từng tool nói rõ:
//!
//! | Nơi ghi | Dùng cho |
//! |---|---|
//! | `update_plan` | các bước của run hiện tại, hết run là mất |
//! | **Bean World** (tools này) | việc làm qua nhiều ngày, có project/assignee/ghi chú |
//! | `schedule_task` | việc chạy định kỳ theo cron |
//! | `memory_save` | kiến thức/kỳ vọng cần nhớ, không phải việc phải làm |
//!
//! `work_project_create` là `Risk::Confirm` (người dùng duyệt — tạo project là quyết
//! định có tác dụng lâu dài); các tool còn lại `Risk::Safe` để agent tự quản task mà
//! không phải xin xác nhận mỗi bước.

use std::sync::Arc;

use async_trait::async_trait;
use bean_tools::typed;
use bean_tools::{Tool, ToolCtx, ToolError};
use bean_types::{Risk, ToolSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::store::Store;

// ---------------------------------------------------------------------------
// Params — doc comment của struct/field chính là description gửi cho model
// ---------------------------------------------------------------------------

/// Liệt kê project trong Bean World (bảng công việc dùng chung với người dùng).
/// Gọi trước khi tạo task để biết `project_id`, hoặc khi người dùng hỏi "đang có
/// project nào".
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkProjectListParams {}

/// Tạo một project trong Bean World — mục tiêu dài hạn chứa nhiều việc, ví dụ
/// "Nâng cấp dashboard". KHÔNG dùng cho việc nhỏ lẻ của phiên hiện tại (hãy dùng
/// `update_plan`), việc chạy định kỳ (dùng `schedule_task`), hay kiến thức cần nhớ
/// (dùng `memory_save`). Người dùng sẽ được hỏi xác nhận trước khi tạo.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkProjectCreateParams {
    /// Tên project, ngắn gọn và duy nhất, ví dụ `bean-world`.
    name: String,
    /// Mô tả mục đích project, có thể rỗng.
    description: String,
}

/// Thêm một việc (task) vào project Bean World. Task mới luôn ở trạng thái `open`.
/// Dùng khi người dùng giao việc cho một subagent (developer, qa, marketing…) hoặc
/// ghi nhận việc cần làm theo dõi qua nhiều ngày.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkTaskAddParams {
    /// ID project chứa việc này (lấy từ `work_project_list`).
    project_id: u64,
    /// Tiêu đề việc, một câu rõ ràng, ví dụ "Viết test cho compaction".
    title: String,
    /// Thông tin liên quan ban đầu (nguồn, tiêu chí xong, link); có thể để rỗng.
    #[serde(default)]
    notes: String,
    /// Role subagent được giao: `developer`, `qa`, `marketing`, `monitor`,
    /// `security-scan`, `finance-readonly`… Để rỗng nếu chưa rõ ai làm.
    #[serde(default)]
    assignee: String,
    /// Mức ưu tiên: `low`, `normal` (mặc định) hoặc `high`.
    #[serde(default = "default_priority")]
    priority: String,
}

fn default_priority() -> String {
    "normal".to_string()
}

/// Liệt kê việc trong Bean World, mới nhất trước; có thể lọc. Dùng khi mở đầu phiên
/// làm việc ("hôm nay còn việc gì dở") hoặc khi người dùng hỏi tiến độ của một project.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkTaskListParams {
    /// Chỉ lấy việc của project này.
    #[serde(default)]
    project_id: Option<u64>,
    /// Chỉ lấy việc giao cho role này (khớp chính xác, ví dụ `developer`).
    #[serde(default)]
    assignee: Option<String>,
    /// Chỉ lấy việc ở trạng thái này: `open` (chưa làm), `in_progress` (đang làm),
    /// `blocked` (bị vướng), `done` (đã xong).
    #[serde(default)]
    status: Option<String>,
}

/// Cập nhật một việc trong Bean World: đổi trạng thái, giao lại việc, đổi priority
/// hay sửa nội dung. Bỏ qua trường nào không muốn đổi. Khi làm xong một việc hãy
/// đặt `status = "done"` (hệ thống tự ghi thời điểm hoàn thành).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkTaskUpdateParams {
    /// ID của việc cần cập nhật (lấy từ `work_task_list`).
    id: u64,
    /// Trạng thái mới: `open`, `in_progress`, `blocked` hoặc `done`.
    #[serde(default)]
    status: Option<String>,
    /// Mức ưu tiên mới: `low`, `normal` hoặc `high`.
    #[serde(default)]
    priority: Option<String>,
    /// Giao việc cho role mới; chuỗi rỗng để huỷ giao.
    #[serde(default)]
    assignee: Option<String>,
    /// Sửa tiêu đề.
    #[serde(default)]
    title: Option<String>,
    /// Ghi đè thông tin liên quan của việc.
    #[serde(default)]
    notes: Option<String>,
}

/// Thêm ghi chú trao đổi (append-only) vào một việc Bean World: tiến độ, vướng mắc,
/// quyết định hay kết quả kiểm tra liên quan tới việc đó. Ghi chú không sửa được —
/// cần đính chính thì thêm ghi chú mới.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkTaskNoteParams {
    /// ID của việc cần thêm ghi chú.
    task_id: u64,
    /// Nội dung ghi chú (tối đa 4000 ký tự), viết thành văn rõ ràng.
    text: String,
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

/// `work_project_list`.
struct WorkProjectListTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for WorkProjectListTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<WorkProjectListParams>("work_project_list")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        typed::deserialize_params::<WorkProjectListParams>(args, &self.spec().parameters)
            .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let projects = self
            .store
            .list_projects()
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        if projects.is_empty() {
            return Ok("Bean World chưa có project nào.".to_string());
        }
        serde_json::to_string(&projects).map_err(|err| ToolError::Internal(err.to_string()))
    }
}

/// `work_project_create` — mức `Confirm`: người dùng duyệt trước khi tạo.
struct WorkProjectCreateTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for WorkProjectCreateTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<WorkProjectCreateParams>("work_project_create")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Confirm
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params =
            typed::deserialize_params::<WorkProjectCreateParams>(args, &self.spec().parameters)
                .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let project = self
            .store
            .create_project(crate::store::NewWorkProject {
                name: params.name,
                description: params.description,
            })
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        serde_json::to_string(&project).map_err(|err| ToolError::Internal(err.to_string()))
    }
}

/// `work_task_add`.
struct WorkTaskAddTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for WorkTaskAddTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<WorkTaskAddParams>("work_task_add")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params = typed::deserialize_params::<WorkTaskAddParams>(args, &self.spec().parameters)
            .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let task = self
            .store
            .create_work_task(crate::store::NewWorkTask {
                project_id: params.project_id,
                title: params.title,
                notes: params.notes,
                assignee: params.assignee,
                priority: params.priority,
                // Tool chạy dưới danh nghĩa agent; REST sẽ ghi `user:<id>` phía trên.
                created_by: "agent".to_string(),
            })
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        serde_json::to_string(&task).map_err(|err| ToolError::Internal(err.to_string()))
    }
}

/// `work_task_list`.
struct WorkTaskListTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for WorkTaskListTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<WorkTaskListParams>("work_task_list")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params = typed::deserialize_params::<WorkTaskListParams>(args, &self.spec().parameters)
            .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let tasks = self
            .store
            .list_work_tasks(crate::store::WorkTaskFilter {
                project_id: params.project_id,
                assignee: params.assignee,
                status: params.status,
            })
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        if tasks.is_empty() {
            return Ok("Không có việc nào khớp bộ lọc.".to_string());
        }
        serde_json::to_string(&tasks).map_err(|err| ToolError::Internal(err.to_string()))
    }
}

/// `work_task_update`.
struct WorkTaskUpdateTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for WorkTaskUpdateTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<WorkTaskUpdateParams>("work_task_update")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params =
            typed::deserialize_params::<WorkTaskUpdateParams>(args, &self.spec().parameters)
                .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let updated = self
            .store
            .update_work_task(
                params.id,
                crate::store::WorkTaskUpdate {
                    title: params.title,
                    notes: params.notes,
                    status: params.status,
                    priority: params.priority,
                    assignee: params.assignee,
                },
            )
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        match updated {
            Some(task) => {
                serde_json::to_string(&task).map_err(|err| ToolError::Internal(err.to_string()))
            }
            None => Ok(format!(
                "task id={} không tồn tại — hãy `work_task_list` để lấy id đúng.",
                params.id
            )),
        }
    }
}

/// `work_task_note`.
struct WorkTaskNoteTool {
    store: Arc<dyn Store>,
}

#[async_trait]
impl Tool for WorkTaskNoteTool {
    fn spec(&self) -> ToolSpec {
        typed::typed_spec::<WorkTaskNoteParams>("work_task_note")
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params = typed::deserialize_params::<WorkTaskNoteParams>(args, &self.spec().parameters)
            .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let note = self
            .store
            .add_work_note(crate::store::NewWorkNote {
                task_id: params.task_id,
                author: "agent".to_string(),
                text: params.text,
            })
            .await
            .map_err(|err| ToolError::Internal(err.to_string()))?;
        serde_json::to_string(&note).map_err(|err| ToolError::Internal(err.to_string()))
    }
}

/// Đăng ký cả sáu tool Bean World dùng **cùng** một store.
#[must_use]
pub fn work_tools(store: Arc<dyn Store>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(WorkProjectListTool {
            store: store.clone(),
        }),
        Arc::new(WorkProjectCreateTool {
            store: store.clone(),
        }),
        Arc::new(WorkTaskAddTool {
            store: store.clone(),
        }),
        Arc::new(WorkTaskListTool {
            store: store.clone(),
        }),
        Arc::new(WorkTaskUpdateTool {
            store: store.clone(),
        }),
        Arc::new(WorkTaskNoteTool { store }),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::work_tools;
    use crate::store::MemoryStore;
    use bean_tools::Risk;
    use std::sync::Arc;

    #[test]
    fn specs_have_names_descriptions_and_risk() {
        let tools = work_tools(Arc::new(MemoryStore::new()));
        let mut names: Vec<String> = tools.iter().map(|tool| tool.spec().name).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "work_project_create",
                "work_project_list",
                "work_task_add",
                "work_task_list",
                "work_task_note",
                "work_task_update",
            ]
        );
        for tool in &tools {
            let spec = tool.spec();
            assert!(
                !spec.description.trim().is_empty(),
                "{} thiếu mô tả",
                spec.name
            );
            assert_eq!(spec.parameters["additionalProperties"], false);
            // Chỉ `work_project_create` cần người dùng duyệt; còn lại Safe để
            // agent tự quản task mà không phải xin xác nhận mỗi bước.
            let expected = if spec.name == "work_project_create" {
                Risk::Confirm
            } else {
                Risk::Safe
            };
            assert_eq!(tool.risk(&serde_json::json!({})), expected);
        }
    }

    // Roundtrip chức năng (project → task → note) nằm ở `store/tests/work.rs`,
    // chạy trên CẢ HAI bản cài đặt — đó mới là nơi giữ bất biến ngữ nghĩa.
}
