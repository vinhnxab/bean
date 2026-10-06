//! Dung axum router: `/api` duoc bao ve, UI co fallback rieng (muc 11.3).
//!
//! # Bat bien khong duoc pham
//!
//! `/api/*` khong ton tai phai tra **404 JSON**, KHONG roo vao SPA fallback (muc
//! 22.17). Neu sai, giao dien hien thi HTML thay vi loi JSON va loi do rat kho chan.
//! Test `web_server.rs` kiem tra dung thu nay.

use axum::Router as AxumRouter;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{any, delete, get, patch, post};

use super::guard::security_headers;
use super::rest_audit_tools::{list_audit, list_mcp_servers, list_tools};
use super::rest_auth_status::{auth_me, list_agents, login, logout, status};
use super::rest_memories::{delete_memory, get_memory_file, list_memories, put_memory_file};
use super::rest_sessions::{
    create_session, delete_session, get_message, list_messages, list_sessions, update_session,
};
use super::rest_skills::{
    approve_skill_draft, get_skill, list_skill_drafts, list_skills, reject_skill_draft,
};
use super::rest_tasks::{create_task, delete_task, list_tasks, update_task};
use super::rest_usage_system::{system_info, usage_history};
use super::ui_assets::{api_not_found, ui_handler};
use super::ws::ws_handler;
use super::{MAX_BODY_BYTES, WebState, request_guard};
/// Dựng toàn bộ axum router; route `/api` được bảo vệ, UI có fallback riêng.
pub fn build_router(state: WebState) -> AxumRouter {
    let api = AxumRouter::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(auth_me))
        .route("/status", get(status))
        .route("/system", get(system_info))
        .route("/tools", get(list_tools))
        .route("/mcp", get(list_mcp_servers))
        .route("/usage", get(usage_history))
        .route("/agents", get(list_agents))
        .route("/sessions", get(list_sessions).post(create_session))
        .route(
            "/sessions/{id}",
            patch(update_session).delete(delete_session),
        )
        .route("/sessions/{id}/messages", get(list_messages))
        .route("/messages/{id}", get(get_message))
        .route(
            "/memory/files/{name}",
            get(get_memory_file).put(put_memory_file),
        )
        .route("/memories", get(list_memories))
        .route("/memories/{id}", delete(delete_memory))
        .route("/skills", get(list_skills))
        .route("/skills/drafts", get(list_skill_drafts))
        .route("/skills/drafts/{id}/approve", post(approve_skill_draft))
        .route("/skills/drafts/{id}/reject", post(reject_skill_draft))
        .route("/skills/{name}", get(get_skill))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", patch(update_task).delete(delete_task))
        .route("/audit", get(list_audit))
        .route("/ws", any(ws_handler))
        .fallback(api_not_found)
        .method_not_allowed_fallback(api_not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES));

    AxumRouter::new()
        .nest("/api", api)
        .fallback(ui_handler)
        .layer(middleware::from_fn_with_state(state.clone(), request_guard))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}
