//! Chuyen record cua store thanh DTO JSON cho REST.
//!
//! # Vai tro
//!
//! Day la **ranh gioi duy nhat** giua kieu noi bo cua `bean-memory` va JSON ra ngoai.
//! Moi record doc tu store deu phai qua day truoc khi ra response — nen mot truong
//! bi xoa o day se khong duoc lo ra bang mot loi compile ma phai lo sang moi handler.

use axum_extra::extract::cookie::CookieJar;
use bean_memory::{MemoryRecord, MessageRecord, ScheduledTask, SessionSummary};

use crate::api_types::{MemoryDto, MessageDto, SESSION_COOKIE, SessionDto, TaskDto};

use super::{ApiFailure, ApiResult, WebState};
/// Lấy token từ cookie, không log giá trị.
pub(crate) fn cookie_token(jar: &CookieJar) -> Option<String> {
    jar.get(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
}

/// Xác thực cookie cho REST handler.
pub(crate) async fn require_user(state: &WebState, jar: &CookieJar) -> ApiResult<String> {
    let token = cookie_token(jar).ok_or_else(ApiFailure::unauthorized)?;
    state
        .auth
        .authenticate_token(&token)
        .await
        .map(|info| info.user_id)
        .map_err(|_| ApiFailure::unauthorized())
}

/// Chuyển `Message` sang DTO JSON cho REST.
pub(crate) fn message_dto(record: MessageRecord) -> ApiResult<MessageDto> {
    let message = serde_json::to_value(record.message).map_err(|_| ApiFailure::internal())?;
    Ok(MessageDto {
        id: record.id,
        session_id: record.session_id.get(),
        seq: record.seq,
        message,
        created_at: record.created_at,
    })
}

pub(crate) fn session_dto(session: SessionSummary) -> SessionDto {
    SessionDto {
        id: session.id.get(),
        channel: session.channel,
        chat_id: session.chat_id,
        user_id: session.user_id,
        title: session.title,
        archived: session.archived,
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

pub(crate) fn memory_dto(memory: MemoryRecord) -> MemoryDto {
    MemoryDto {
        id: memory.id,
        text: memory.text,
        tags: memory.tags,
        created_at: memory.created_at,
    }
}

pub(crate) fn task_dto(task: ScheduledTask) -> TaskDto {
    TaskDto {
        id: task.id,
        session_id: task.session_id.map_or(0, |id| id.get()),
        cron: task.cron,
        prompt: task.prompt,
        channel: task.channel,
        chat_id: task.chat_id,
        allowed_tools: task.allowed_tools,
        next_run: task.next_run,
        enabled: task.enabled,
        created_at: task.created_at,
        last_run_at: task.last_run_at,
        last_status: task.last_status,
    }
}
