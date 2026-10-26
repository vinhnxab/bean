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
use axum::routing::{any, delete, get, patch, post, put};
use tower::Layer;
use tower_http::services::{ServeDir, ServeFile};

use super::guard::security_headers;
use super::rest_audit_tools::{list_audit, list_mcp_servers, list_tools, set_tool_enabled};
use super::rest_auth_status::{auth_me, list_agents, login, logout, status};
use super::rest_config::{
    create_llm_provider_profile, delete_llm_provider_profile, get_llm_config, update_llm_model,
    update_llm_provider, update_llm_provider_profile,
};
use super::rest_memories::{delete_memory, get_memory_file, list_memories, put_memory_file};
use super::rest_sessions::{
    create_session, delete_session, get_message, list_messages, list_sessions, update_session,
};
use super::rest_skills::{
    approve_skill_draft, get_skill, list_skill_drafts, list_skills, reject_skill_draft,
};
use super::rest_tasks::{create_task, delete_task, list_tasks, update_task};
use super::rest_usage_system::{system_info, usage_history};
use super::rest_voice::{voice_status, voice_synthesize, voice_transcribe, voice_update};
use super::ui_assets::{StaticRoot, api_not_found, cache_control_layer, ui_handler};
use super::ws::ws_handler;
use super::{MAX_BODY_BYTES, VOICE_TRANSCRIBE_BODY_BYTES, WebState, request_guard};
/// Dựng toàn bộ axum router; route `/api` được bảo vệ, UI có fallback riêng.
pub fn build_router(state: WebState) -> AxumRouter {
    let api = AxumRouter::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(auth_me))
        .route("/status", get(status))
        .route("/system", get(system_info))
        .route("/config/llm", get(get_llm_config))
        .route("/config/llm/model", put(update_llm_model))
        .route("/config/llm/provider", put(update_llm_provider))
        .route("/config/llm/providers", post(create_llm_provider_profile))
        .route(
            "/config/llm/providers/{name}",
            put(update_llm_provider_profile).delete(delete_llm_provider_profile),
        )
        .route("/tools", get(list_tools))
        .route("/tools/{name}/enabled", put(set_tool_enabled))
        .route("/mcp", get(list_mcp_servers))
        .route("/usage", get(usage_history))
        .route("/voice", get(voice_status).patch(voice_update))
        // WAV 16 kHz nặng ~32 KB/giây (base64 ~43 KB/s) — nới riêng trần body cho
        // route này, các route khác vẫn giữ trần chung `MAX_BODY_BYTES`.
        .route(
            "/voice/transcribe",
            post(voice_transcribe).layer(DefaultBodyLimit::max(VOICE_TRANSCRIBE_BODY_BYTES)),
        )
        .route("/voice/synthesize", post(voice_synthesize))
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

    // `[web].ui_dir` thắng bản nhúng: người dùng chủ ý trỏ UI sang thư mục riêng
    // để nâng cấp giao diện không cần build lại binary (mục 11.3).
    let router = AxumRouter::new().nest("/api", api);
    let router = match StaticRoot::from_config(&state.config) {
        StaticRoot::Dir(dir) => {
            if !dir.is_dir() {
                tracing::warn!(
                    dir = %dir.display(),
                    "web.ui_dir không tồn tại — giao diện từ đĩa sẽ trả 404 cho tới khi thư mục được tạo"
                );
            }
            router
                .nest_service(
                    "/assets",
                    cache_control_layer("public, max-age=31536000, immutable")
                        .layer(ServeDir::new(dir.join("assets"))),
                )
                .fallback_service(
                    cache_control_layer("no-cache").layer(
                        ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html"))),
                    ),
                )
        }
        StaticRoot::Embedded | StaticRoot::None => router.fallback(ui_handler),
    };

    router
        .layer(middleware::from_fn_with_state(state.clone(), request_guard))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}
