//! Handler REST: tac vu dinh ky (muc 14).
//!
//! # Bat bien
//!
//! `task_owned_by_user` chuyen quyen duyet: chi nguoi da tao task moi sua/xoa duoc.
//! `cron` duoc validate va `next_run` tinh theo **múi giờ nguoi dung** nhung luu UTC
//! (muc 14).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_core::next_run_after;
use bean_memory::{NewScheduledTask, ScheduledTask};
use bean_types::SessionId;

use crate::api_types::{DeleteResponse, TaskDto, TaskListResponse, TaskRequest, TaskUpdateRequest};

use super::{ApiFailure, ApiResult, WebState, require_user, task_dto};
pub(super) async fn task_owned_by_user(
    state: &WebState,
    task: &ScheduledTask,
    user_id: &str,
) -> ApiResult<bool> {
    let Some(session) = task.session_id else {
        return Ok(false);
    };
    let info = state
        .store
        .session_info(session)
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(info.is_some_and(|info| info.user_id == user_id))
}

pub(super) async fn list_tasks(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<TaskListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let tasks = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?;
    let mut result = Vec::new();
    for task in tasks {
        if task_owned_by_user(&state, &task, &user_id).await? {
            result.push(task_dto(task));
        }
    }
    Ok(Json(TaskListResponse { tasks: result }))
}

pub(super) async fn create_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<TaskRequest>,
) -> ApiResult<(StatusCode, Json<TaskDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let cron = request.cron.trim().to_string();
    let prompt = request.prompt.trim().to_string();
    let channel = request.channel.trim().to_string();
    let chat_id = request.chat_id.trim().to_string();
    if cron.is_empty() || prompt.is_empty() || channel.is_empty() || chat_id.is_empty() {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_task",
            "cron, prompt, channel và chat_id không được để trống",
        ));
    }
    if cron.len() > 256 || prompt.len() > 16_000 || channel.len() > 64 || chat_id.len() > 256 {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_task",
            "một trường của tác vụ quá dài",
        ));
    }
    let session = match request.session_id {
        Some(id) => {
            let info = state
                .store
                .session_info(SessionId::new(id))
                .await
                .map_err(|_| ApiFailure::internal())?
                .ok_or_else(ApiFailure::not_found)?;
            if info.user_id != user_id
                || info.channel != channel
                || info.chat_id != chat_id
                || info.archived
            {
                return Err(ApiFailure::not_found());
            }
            SessionId::new(id)
        }
        None => {
            if channel != "web" {
                return Err(ApiFailure::new(
                    StatusCode::BAD_REQUEST,
                    "session_required",
                    "task ngoài web phải chỉ định session_id thuộc user",
                ));
            }
            let active = state
                .store
                .find_active_session(&channel, &chat_id)
                .await
                .map_err(|_| ApiFailure::internal())?;
            if let Some(active) = active {
                let info = state
                    .store
                    .session_info(active)
                    .await
                    .map_err(|_| ApiFailure::internal())?;
                if info.is_some_and(|info| info.user_id == user_id && !info.archived) {
                    active
                } else {
                    state
                        .store
                        .ensure_session_for_user(&channel, &chat_id, &user_id, "")
                        .await
                        .map_err(|_| ApiFailure::internal())?
                }
            } else {
                state
                    .store
                    .ensure_session_for_user(&channel, &chat_id, &user_id, "")
                    .await
                    .map_err(|_| ApiFailure::internal())?
            }
        }
    };
    let allowed_tools = request
        .allowed_tools
        .into_iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect();
    let next_run = next_run_after(&cron, &state.config.agent.timezone, chrono::Utc::now())
        .map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_cron",
                "biểu thức cron không hợp lệ",
            )
        })?
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let task = state
        .store
        .create_task(NewScheduledTask {
            cron,
            prompt,
            session_id: Some(session),
            channel,
            chat_id,
            allowed_tools,
            next_run,
            enabled: request.enabled.unwrap_or(true),
        })
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok((StatusCode::CREATED, Json(task_dto(task))))
}

pub(super) async fn update_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
    Json(request): Json<TaskUpdateRequest>,
) -> ApiResult<Json<TaskDto>> {
    let user_id = require_user(&state, &jar).await?;
    let existing = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .find(|task| task.id == id)
        .ok_or_else(ApiFailure::not_found)?;
    if !task_owned_by_user(&state, &existing, &user_id).await? {
        return Err(ApiFailure::not_found());
    }
    let task = state
        .store
        .set_task_enabled(id, request.enabled)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::not_found)?;
    Ok(Json(task_dto(task)))
}

pub(super) async fn delete_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
) -> ApiResult<Json<DeleteResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let existing = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .find(|task| task.id == id)
        .ok_or_else(ApiFailure::not_found)?;
    if !task_owned_by_user(&state, &existing, &user_id).await? {
        return Err(ApiFailure::not_found());
    }
    let deleted = state
        .store
        .delete_task(id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !deleted {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(DeleteResponse { deleted }))
}
