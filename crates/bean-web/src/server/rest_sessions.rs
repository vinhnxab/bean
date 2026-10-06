//! Handler REST: phien hoi thoai va message.
//!
//! # Chong IDOR
//!
//! Moi lan doc/ghi deu kiem tra session co thuoc user dang goi (`user_id` tu cookie)
//! truoc khi thao tac - adapter web khong duoc tin session id tu client (muc 10).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_types::SessionId;

use crate::api_types::{
    CreateSessionRequest, DeleteResponse, MessageDto, MessageListResponse, MessageQuery,
    OkResponse, SessionDto, SessionListResponse, SessionQuery, UpdateSessionRequest,
};

use super::{ApiFailure, ApiResult, WebState, message_dto, require_user, session_dto};

pub(super) async fn list_sessions(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<SessionQuery>,
) -> ApiResult<Json<SessionListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let sessions = state
        .store
        .list_sessions(
            &user_id,
            query.q.as_deref(),
            query.archived,
            query.limit.unwrap_or(100).min(500),
        )
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(SessionListResponse {
        sessions: sessions.into_iter().map(session_dto).collect(),
    }))
}

pub(super) async fn create_session(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let title = request.title.unwrap_or_default();
    let session = state
        .store
        .create_session("web", &user_id, &user_id, &title)
        .await
        .map_err(|_| ApiFailure::internal())?;
    let summary = state
        .store
        .session_summary(session)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::internal)?;
    Ok((StatusCode::CREATED, Json(session_dto(summary))))
}

pub(super) async fn update_session(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Json(request): Json<UpdateSessionRequest>,
) -> ApiResult<Json<OkResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let changed = state
        .store
        .update_session(
            SessionId::new(id),
            &user_id,
            request.title.as_deref(),
            request.archived,
        )
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !changed {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(OkResponse { ok: true }))
}

pub(super) async fn delete_session(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> ApiResult<Json<DeleteResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let deleted = state
        .store
        .delete_session(SessionId::new(id), &user_id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(DeleteResponse { deleted }))
}

pub(super) async fn list_messages(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<MessageListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let info = state
        .store
        .session_info(SessionId::new(id))
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !info.is_some_and(|item| item.user_id == user_id && item.channel == "web") {
        return Err(ApiFailure::not_found());
    }
    let records = state
        .store
        .list_message_records(
            SessionId::new(id),
            query.before,
            query.limit.unwrap_or(100).min(500),
        )
        .await
        .map_err(|_| ApiFailure::internal())?;
    let messages = records
        .into_iter()
        .map(message_dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(MessageListResponse { messages }))
}

pub(super) async fn get_message(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> ApiResult<Json<MessageDto>> {
    let user_id = require_user(&state, &jar).await?;
    let record = state
        .store
        .message_by_id(id)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::not_found)?;
    let info = state
        .store
        .session_info(record.session_id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !info.is_some_and(|item| item.user_id == user_id && item.channel == "web") {
        return Err(ApiFailure::not_found());
    }
    message_dto(record).map(Json)
}
