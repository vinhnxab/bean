//! `WorkStore` của [`MemoryStore`] — Bean World (Project → Task → Note) tách khỏi
//! `memory.rs` để giữ đúng trần số dòng mà test kiến trúc canh cho thư mục `store/`
//! (pattern của `memory_push.rs`).
//!
//! Ngữ nghĩa phải **khớp SqliteStore** (`sqlite/q_bw_task.rs`…): sort ưu tiên
//! priority rồi mới nhất, `done_at` đặt khi vào `done`/xoá khi rời `done`, xoá
//! task kéo theo ghi chú. Test dùng chung trong `tests/work.rs` là ràng buộc này.

use super::memory::MemoryStore;
use super::shared::now_rfc3339;
use super::sqlite::q_bw;
use super::types::{
    NewWorkNote, NewWorkProject, NewWorkTask, StoreError, WorkNote, WorkProject, WorkTask,
    WorkTaskFilter, WorkTaskUpdate,
};
use crate::traits::traits::WorkStore;

#[async_trait::async_trait]
impl WorkStore for MemoryStore {
    async fn list_projects(&self) -> Result<Vec<WorkProject>, StoreError> {
        let projects = self.work_projects.read().await;
        let mut result: Vec<WorkProject> = projects.clone();
        result.sort_by_key(|project| std::cmp::Reverse(project.id));
        Ok(result)
    }

    async fn create_project(&self, project: NewWorkProject) -> Result<WorkProject, StoreError> {
        let name = project.name.trim().to_string();
        if name.is_empty() || name.chars().count() > q_bw::PROJECT_NAME_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "tên project phải 1..={} ký tự",
                q_bw::PROJECT_NAME_MAX_CHARS
            )));
        }
        let description = project.description.trim().to_string();
        if description.chars().count() > q_bw::NOTES_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "mô tả project tối đa {} ký tự",
                q_bw::NOTES_MAX_CHARS
            )));
        }
        let mut projects = self.work_projects.write().await;
        if projects.iter().any(|p| p.name == name) {
            return Err(StoreError::Internal(format!("project `{name}` đã tồn tại")));
        }
        let id = {
            let mut next = self.next_work_project_id.write().await;
            let value = *next;
            *next = next.saturating_add(1);
            value
        };
        let now = now_rfc3339();
        let record = WorkProject {
            id,
            name,
            description,
            status: "active".into(),
            created_at: now.clone(),
            updated_at: now,
        };
        projects.push(record.clone());
        Ok(record)
    }

    async fn set_project_status(
        &self,
        id: u64,
        status: &str,
    ) -> Result<Option<WorkProject>, StoreError> {
        if status != "active" && status != "archived" {
            return Err(StoreError::Internal(
                "trạng thái project chỉ nhận: active, archived".into(),
            ));
        }
        let mut projects = self.work_projects.write().await;
        let Some(project) = projects.iter_mut().find(|project| project.id == id) else {
            return Ok(None);
        };
        project.status = status.to_string();
        project.updated_at = now_rfc3339();
        Ok(Some(project.clone()))
    }

    async fn list_work_tasks(&self, filter: WorkTaskFilter) -> Result<Vec<WorkTask>, StoreError> {
        if let Some(status) = &filter.status {
            q_bw::validate_status(status)?;
        }
        let tasks = self.work_tasks.read().await;
        let mut result: Vec<WorkTask> = tasks
            .iter()
            .filter(|task| match filter.project_id {
                Some(project_id) => task.project_id == project_id,
                None => true,
            })
            .filter(|task| match &filter.status {
                Some(status) => &task.status == status,
                None => true,
            })
            .filter(|task| match &filter.assignee {
                Some(assignee) => &task.assignee == assignee,
                None => true,
            })
            .cloned()
            .collect();
        result.sort_by_key(|task| std::cmp::Reverse(task.id));
        Ok(result)
    }

    async fn create_work_task(&self, task: NewWorkTask) -> Result<WorkTask, StoreError> {
        let title = task.title.trim().to_string();
        if title.is_empty() || title.chars().count() > q_bw::TASK_TITLE_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "tiêu đề task phải 1..={} ký tự",
                q_bw::TASK_TITLE_MAX_CHARS
            )));
        }
        let notes = task.notes.trim().to_string();
        if notes.chars().count() > q_bw::NOTES_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "notes tối đa {} ký tự",
                q_bw::NOTES_MAX_CHARS
            )));
        }
        q_bw::validate_priority(&task.priority)?;
        let mut tasks = self.work_tasks.write().await;
        let project_exists = self
            .work_projects
            .read()
            .await
            .iter()
            .any(|project| project.id == task.project_id);
        if !project_exists {
            return Err(StoreError::Internal(format!(
                "project id={} không tồn tại — hãy `project_list` trước khi thêm task",
                task.project_id
            )));
        }
        let id = {
            let mut next = self.next_work_task_id.write().await;
            let value = *next;
            *next = next.saturating_add(1);
            value
        };
        let now = now_rfc3339();
        let record = WorkTask {
            id,
            project_id: task.project_id,
            title,
            notes,
            status: "open".into(),
            priority: task.priority,
            assignee: task.assignee.trim().to_string(),
            created_by: task.created_by,
            created_at: now.clone(),
            updated_at: now,
            done_at: None,
        };
        tasks.push(record.clone());
        Ok(record)
    }

    async fn update_work_task(
        &self,
        id: u64,
        update: WorkTaskUpdate,
    ) -> Result<Option<WorkTask>, StoreError> {
        if let Some(title) = &update.title {
            let title = title.trim();
            if title.is_empty() || title.chars().count() > q_bw::TASK_TITLE_MAX_CHARS {
                return Err(StoreError::Internal(format!(
                    "tiêu đề task phải 1..={} ký tự",
                    q_bw::TASK_TITLE_MAX_CHARS
                )));
            }
        }
        if let Some(notes) = &update.notes
            && notes.chars().count() > q_bw::NOTES_MAX_CHARS
        {
            return Err(StoreError::Internal(format!(
                "notes tối đa {} ký tự",
                q_bw::NOTES_MAX_CHARS
            )));
        }
        if let Some(status) = &update.status {
            q_bw::validate_status(status)?;
        }
        if let Some(priority) = &update.priority {
            q_bw::validate_priority(priority)?;
        }
        let mut tasks = self.work_tasks.write().await;
        let Some(task) = tasks.iter_mut().find(|task| task.id == id) else {
            return Ok(None);
        };
        if let Some(title) = &update.title {
            task.title = title.trim().to_string();
        }
        if let Some(notes) = &update.notes {
            task.notes = notes.trim().to_string();
        }
        if let Some(status) = &update.status {
            task.status = status.clone();
            // Khớp q_bw_task: vào `done` đóng dấu thời điểm, rời `done` xoá dấu.
            task.done_at = (status == "done").then(now_rfc3339);
        }
        if let Some(priority) = &update.priority {
            task.priority = priority.clone();
        }
        if let Some(assignee) = &update.assignee {
            task.assignee = assignee.trim().to_string();
        }
        task.updated_at = now_rfc3339();
        Ok(Some(task.clone()))
    }

    async fn delete_work_task(&self, id: u64) -> Result<bool, StoreError> {
        let mut tasks = self.work_tasks.write().await;
        let before = tasks.len();
        tasks.retain(|task| task.id != id);
        let deleted = tasks.len() != before;
        if deleted {
            // Khớp FK CASCADE của SQLite: xoá task phải kéo theo ghi chú của nó,
            // nếu không bản in-memory giữ "note ma" trỏ về task đã mất.
            let mut notes = self.work_notes.write().await;
            notes.retain(|note| note.task_id != id);
        }
        Ok(deleted)
    }

    async fn add_work_note(&self, note: NewWorkNote) -> Result<WorkNote, StoreError> {
        let text = note.text.trim().to_string();
        if text.is_empty() || text.chars().count() > q_bw::NOTES_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "ghi chú phải 1..={} ký tự",
                q_bw::NOTES_MAX_CHARS
            )));
        }
        let mut tasks = self.work_tasks.write().await;
        let Some(task) = tasks.iter_mut().find(|task| task.id == note.task_id) else {
            return Err(StoreError::Internal(format!(
                "task id={} không tồn tại",
                note.task_id
            )));
        };
        let id = {
            let mut next = self.next_work_note_id.write().await;
            let value = *next;
            *next = next.saturating_add(1);
            value
        };
        let now = now_rfc3339();
        let record = WorkNote {
            id,
            task_id: note.task_id,
            author: note.author,
            text,
            created_at: now.clone(),
        };
        self.work_notes.write().await.push(record.clone());
        // Task có hoạt động mới: đẩy updated_at để board xếp lại theo sửa đổi.
        task.updated_at = now;
        Ok(record)
    }

    async fn list_work_notes(&self, task_id: u64) -> Result<Vec<WorkNote>, StoreError> {
        let notes = self.work_notes.read().await;
        let mut result: Vec<WorkNote> = notes
            .iter()
            .filter(|note| note.task_id == task_id)
            .cloned()
            .collect();
        result.sort_by_key(|note| note.id);
        Ok(result)
    }

    async fn open_work_tasks(&self, limit: usize) -> Result<Vec<WorkTask>, StoreError> {
        let limit = limit.clamp(1, 50);
        let tasks = self.work_tasks.read().await;
        let mut result: Vec<WorkTask> = tasks
            .iter()
            .filter(|task| matches!(task.status.as_str(), "open" | "in_progress" | "blocked"))
            .cloned()
            .collect();
        result.sort_by(|left, right| {
            // Khớp ORDER BY của SQLite: priority high → normal → low, cùng priority thì id mới trước.
            let priority_rank = |priority: &str| match priority {
                "high" => 0,
                "normal" => 1,
                _ => 2,
            };
            priority_rank(&left.priority)
                .cmp(&priority_rank(&right.priority))
                .then_with(|| right.id.cmp(&left.id))
        });
        result.truncate(limit);
        Ok(result)
    }
}
