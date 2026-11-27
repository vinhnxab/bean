//! Handler REST Bean World — bảng công việc `Project → Task → Note` dùng chung
//! giữa người thật (UI ở đây) và agent (tools `work_*`). Một nguồn sự thật nằm ở
//! store; handler chỉ validate đầu vào ở **ranh giới** rồi gọi store.
//!
//! # Ranh giớivalidate
//!
//! Luật (trần ký tự, tập giá trị) lấy trực tiếp từ `bean_memory` (re-export của
//! `q_bw`) để tool, REST và store không lệch nhau. Lỗi đầu vào trả `400` với
//! `code` đọc được (bám `invalid_task`) để client — người lẫn model — tự sửa.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_memory::{
    NewWorkNote, NewWorkProject, NewWorkTask, WORK_NOTES_MAX_CHARS, WORK_PROJECT_NAME_MAX_CHARS,
    WORK_TASK_TITLE_MAX_CHARS, WorkTaskFilter, WorkTaskUpdate, validate_work_priority,
    validate_work_status,
};
use serde::Deserialize;

use crate::api_types::{
    DeleteResponse, WorkNoteCreateRequest, WorkNoteDto, WorkNoteListResponse,
    WorkProjectCreateRequest, WorkProjectDto, WorkProjectListResponse, WorkProjectStatusRequest,
    WorkTaskCreateRequest, WorkTaskDto, WorkTaskListResponse, WorkTaskUpdateRequest,
};

use super::{ApiFailure, ApiResult, WebState, require_user};

// ---------------------------------------------------------------------------
// Converter record → DTO
// ---------------------------------------------------------------------------

fn work_project_dto(project: bean_memory::WorkProject) -> WorkProjectDto {
    WorkProjectDto {
        id: project.id,
        name: project.name,
        description: project.description,
        status: project.status,
        created_at: project.created_at,
        updated_at: project.updated_at,
    }
}

fn work_task_dto(task: bean_memory::WorkTask) -> WorkTaskDto {
    WorkTaskDto {
        id: task.id,
        project_id: task.project_id,
        title: task.title,
        notes: task.notes,
        status: task.status,
        priority: task.priority,
        assignee: task.assignee,
        created_by: task.created_by,
        created_at: task.created_at,
        updated_at: task.updated_at,
        done_at: task.done_at,
    }
}

fn work_note_dto(note: bean_memory::WorkNote) -> WorkNoteDto {
    WorkNoteDto {
        id: note.id,
        task_id: note.task_id,
        author: note.author,
        text: note.text,
        created_at: note.created_at,
    }
}

/// Lỗi validate đầu vào — 400 với `code` đọc được.
fn invalid(message: impl Into<String>) -> ApiFailure {
    ApiFailure::new(StatusCode::BAD_REQUEST, "invalid_work", message)
}

/// Kiểm tra chuỗi ký tự Unicode trong trần cho phép; trả câu lỗi đọc được.
fn check_len(field: &str, value: &str, max: usize) -> Result<(), ApiFailure> {
    let count = value.chars().count();
    if count > max {
        return Err(invalid(format!("{field} tối đa {max} ký tự")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

/// `GET /api/work/projects`.
pub(super) async fn list_work_projects(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<WorkProjectListResponse>> {
    let _user = require_user(&state, &jar).await?;
    let projects = state
        .store
        .list_projects()
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(WorkProjectListResponse {
        projects: projects.into_iter().map(work_project_dto).collect(),
    }))
}

/// `POST /api/work/projects` — tương đương tool `work_project_create` (agent phải
/// xin duyệt; người thật qua UI tự tạo trực tiếp).
pub(super) async fn create_work_project(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<WorkProjectCreateRequest>,
) -> ApiResult<(StatusCode, Json<WorkProjectDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(invalid("tên project không được để trống"));
    }
    check_len("tên project", &name, WORK_PROJECT_NAME_MAX_CHARS)?;
    let description = request.description.trim().to_string();
    check_len("mô tả project", &description, WORK_NOTES_MAX_CHARS)?;
    let project = state
        .store
        .create_project(NewWorkProject { name, description })
        .await
        .map_err(|err| match err {
            // Trùng tên là lỗi tham số của người gọi, không phải lỗi hệ thống.
            bean_memory::StoreError::Internal(message) if message.contains("đã tồn tại") => {
                invalid(message)
            }
            _ => ApiFailure::internal(),
        })?;
    tracing::info!(user = %user_id, project_id = project.id, "tạo project Bean World");
    Ok((StatusCode::CREATED, Json(work_project_dto(project))))
}

/// `PATCH /api/work/projects/{id}` — archive/active lại project.
pub(super) async fn update_work_project(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
    Json(request): Json<WorkProjectStatusRequest>,
) -> ApiResult<Json<WorkProjectDto>> {
    let _user = require_user(&state, &jar).await?;
    if request.status != "active" && request.status != "archived" {
        return Err(invalid("trạng thái project chỉ nhận: active, archived"));
    }
    let project = state
        .store
        .set_project_status(id, &request.status)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::not_found)?;
    Ok(Json(work_project_dto(project)))
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/// Query của `GET /api/work/tasks`.
#[derive(Debug, Default, Deserialize)]
pub(super) struct WorkTaskQuery {
    /// Lọc theo project.
    project_id: Option<u64>,
    /// Lọc theo role được giao.
    assignee: Option<String>,
    /// Lọc theo trạng thái.
    status: Option<String>,
}

/// `GET /api/work/tasks?project_id=&assignee=&status=`.
pub(super) async fn list_work_tasks(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<WorkTaskQuery>,
) -> ApiResult<Json<WorkTaskListResponse>> {
    let _user = require_user(&state, &jar).await?;
    if let Some(status) = &query.status {
        validate_work_status(status).map_err(|err| invalid(err.to_string()))?;
    }
    let tasks = state
        .store
        .list_work_tasks(WorkTaskFilter {
            project_id: query.project_id,
            assignee: query.assignee,
            status: query.status,
        })
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(WorkTaskListResponse {
        tasks: tasks.into_iter().map(work_task_dto).collect(),
    }))
}

/// `POST /api/work/tasks`.
pub(super) async fn create_work_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<WorkTaskCreateRequest>,
) -> ApiResult<(StatusCode, Json<WorkTaskDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err(invalid("tiêu đề task không được để trống"));
    }
    check_len("tiêu đề task", &title, WORK_TASK_TITLE_MAX_CHARS)?;
    let notes = request.notes.trim().to_string();
    check_len("notes", &notes, WORK_NOTES_MAX_CHARS)?;
    validate_work_priority(&request.priority).map_err(|err| invalid(err.to_string()))?;
    // Project phải tồn tại: báo `not_found` rõ ràng thay vì để FK từ chối 500.
    let project = state
        .store
        .list_projects()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .find(|project| project.id == request.project_id);
    if project.is_none() {
        return Err(ApiFailure::not_found());
    }
    let task = state
        .store
        .create_work_task(NewWorkTask {
            project_id: request.project_id,
            title,
            notes,
            assignee: request.assignee.trim().to_string(),
            priority: request.priority,
            // Người thật tạo qua UI; agent tạo qua tool sẽ ghi `agent`.
            created_by: format!("user:{user_id}"),
        })
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok((StatusCode::CREATED, Json(work_task_dto(task))))
}

/// `PATCH /api/work/tasks/{id}`.
pub(super) async fn update_work_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
    Json(request): Json<WorkTaskUpdateRequest>,
) -> ApiResult<Json<WorkTaskDto>> {
    let _user = require_user(&state, &jar).await?;
    if let Some(title) = &request.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(invalid("tiêu đề task không được để trống"));
        }
        check_len("tiêu đề task", title, WORK_TASK_TITLE_MAX_CHARS)?;
    }
    if let Some(notes) = &request.notes {
        check_len("notes", notes, WORK_NOTES_MAX_CHARS)?;
    }
    if let Some(status) = &request.status {
        validate_work_status(status).map_err(|err| invalid(err.to_string()))?;
    }
    if let Some(priority) = &request.priority {
        validate_work_priority(priority).map_err(|err| invalid(err.to_string()))?;
    }
    let task = state
        .store
        .update_work_task(
            id,
            WorkTaskUpdate {
                title: request.title,
                notes: request.notes,
                status: request.status,
                priority: request.priority,
                assignee: request.assignee.map(|assignee| assignee.trim().to_string()),
            },
        )
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::not_found)?;
    Ok(Json(work_task_dto(task)))
}

/// `DELETE /api/work/tasks/{id}` — ghi chú gắn task bị xoá theo (FK CASCADE).
pub(super) async fn delete_work_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
) -> ApiResult<Json<DeleteResponse>> {
    let _user = require_user(&state, &jar).await?;
    let deleted = state
        .store
        .delete_work_task(id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !deleted {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(DeleteResponse { deleted }))
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

/// `GET /api/work/tasks/{id}/notes`.
pub(super) async fn list_work_notes(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(task_id): Path<u64>,
) -> ApiResult<Json<WorkNoteListResponse>> {
    let _user = require_user(&state, &jar).await?;
    let notes = state
        .store
        .list_work_notes(task_id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(WorkNoteListResponse {
        notes: notes.into_iter().map(work_note_dto).collect(),
    }))
}

/// `POST /api/work/tasks/{id}/notes`.
pub(super) async fn add_work_note(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(task_id): Path<u64>,
    Json(request): Json<WorkNoteCreateRequest>,
) -> ApiResult<(StatusCode, Json<WorkNoteDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let text = request.text.trim().to_string();
    if text.is_empty() {
        return Err(invalid("ghi chú không được để trống"));
    }
    check_len("ghi chú", &text, WORK_NOTES_MAX_CHARS)?;
    let note = state
        .store
        .add_work_note(NewWorkNote {
            task_id,
            author: format!("user:{user_id}"),
            text,
        })
        .await
        .map_err(|err| match err {
            bean_memory::StoreError::Internal(message) if message.contains("không tồn tại") => {
                ApiFailure::not_found()
            }
            _ => ApiFailure::internal(),
        })?;
    Ok((StatusCode::CREATED, Json(work_note_dto(note))))
}
