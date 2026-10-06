//! Handler REST: bo nho dai han + `MEMORY.md`/`USER.md`.
//!
//! # Duong dan phai qua path jail
//!
//! `get_memory_file`/`put_memory_file` chi nhan `MEMORY` hoac `USER`, khong nhan ten
//! tep tu client — do la lop bao ve path jail o tang adapter (muc 15.1).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum_extra::extract::cookie::CookieJar;

use crate::api_types::{
    DeleteResponse, MemoryFileRequest, MemoryFileResponse, MemoryListResponse, MemoryQuery,
};

use super::{ApiFailure, ApiResult, WebState, memory_dto, require_user};
pub(super) async fn list_memories(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<MemoryQuery>,
) -> ApiResult<Json<MemoryListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let memories = state
        .store
        .list_memories(query.q.as_deref(), query.limit.unwrap_or(100).min(500))
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(MemoryListResponse {
        memories: memories.into_iter().map(memory_dto).collect(),
    }))
}

pub(super) async fn delete_memory(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
) -> ApiResult<Json<DeleteResponse>> {
    let _ = require_user(&state, &jar).await?;
    let deleted = state
        .store
        .delete_memory(id)
        .await
        .map_err(|error| match error {
            bean_memory::StoreError::NotFound(_) => ApiFailure::not_found(),
            _ => ApiFailure::internal(),
        })?;
    Ok(Json(DeleteResponse { deleted }))
}

pub(super) async fn get_memory_file(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
) -> ApiResult<Json<MemoryFileResponse>> {
    let _ = require_user(&state, &jar).await?;
    let file = match name.as_str() {
        "MEMORY" | "MEMORY.md" => "MEMORY.md",
        "USER" | "USER.md" => "USER.md",
        _ => return Err(ApiFailure::not_found()),
    };
    let workspace = state.workspace.as_ref().ok_or_else(ApiFailure::internal)?;
    let content = tokio::task::spawn_blocking({
        let workspace = workspace.clone();
        let file = file.to_string();
        move || workspace.read_text(&file)
    })
    .await
    .map_err(|_| ApiFailure::internal())?
    .map_err(|error| match error {
        bean_tools::ToolError::NotFound(_) => ApiFailure::not_found(),
        _ => ApiFailure::internal(),
    })?;
    Ok(Json(MemoryFileResponse { name, content }))
}

pub(super) async fn put_memory_file(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
    Json(request): Json<MemoryFileRequest>,
) -> ApiResult<Json<MemoryFileResponse>> {
    let _ = require_user(&state, &jar).await?;
    let file = match name.as_str() {
        "MEMORY" | "MEMORY.md" => "MEMORY.md",
        "USER" | "USER.md" => "USER.md",
        _ => return Err(ApiFailure::not_found()),
    };
    let workspace = state.workspace.as_ref().ok_or_else(ApiFailure::internal)?;
    tokio::task::spawn_blocking({
        let workspace = workspace.clone();
        let file = file.to_string();
        let content = request.content.clone();
        move || workspace.write_text(&file, &content).map_err(|_| ())
    })
    .await
    .map_err(|_| ApiFailure::internal())?
    .map_err(|_| ApiFailure::internal())?;
    Ok(Json(MemoryFileResponse {
        name,
        content: request.content,
    }))
}
