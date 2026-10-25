//! Black-box tests cho runtime toggle tool: `PUT /api/tools/{name}/enabled`.
//!
//! # Trọng tâm
//!
//! Toggle là hành động **thay đổi quyền năng của agent lúc chạy** nên phải:
//! * chỉ admin được gọi (403 cho role thường, 401 cho người chưa đăng nhập);
//! * trả 404 cho tool không tồn tại (không âm thầm trả Ok của tool khác);
//! * persist vào store **trước** khi mutate registry — store là nguồn sự thật lúc
//!   khởi động lại, registry chỉ là cache runtime;
//! * mỗi lần toggle phải ghi audit với `decided_by` là người đã ra quyết định.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use bean_core::{Router, RouterDeps};
use bean_llm::FakeProvider;
use bean_memory::{MemoryStore, Store, ToolOverride};
use bean_security::CapWorkspace;
use bean_skills::SkillCatalog;
use bean_tools::{ToolRegistry, builtin::files::tool::read_file};
use bean_types::config::RoleConfig;
use bean_types::{Config, LlmResponse};
use bean_web::{AuthService, WebState, WebStateDeps, build_router, set_password};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";
const ORIGIN: &str = "http://127.0.0.1:7878";

fn role(name: &str, tags: &[&str]) -> RoleConfig {
    RoleConfig {
        name: name.to_string(),
        tool_tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        forbid_tags: Vec::new(),
        allowed_tool_tags: Vec::new(),
        context_budget_tokens: None,
        daily_token_budget: None,
    }
}

/// RBAC tắt khi `viewer` là `None` (một người dùng); bật khi có `(user, role)`.
fn config_for(data: &Path, workspace: &Path, viewer: Option<(&str, &str)>) -> Config {
    let mut config = Config::default();
    config.data.dir = data.to_path_buf();
    config.agent.workspace = workspace.to_path_buf();
    config.web.public_origin = ORIGIN.to_string();
    config.web.session_ttl_hours = 1;
    if let Some((user, role_name)) = viewer {
        config
            .agent
            .user_roles
            .insert(user.to_string(), role_name.to_string());
        config.agent.allowed_users = vec![user.to_string()];
        config.web.user_id = user.to_string();
        config.roles = vec![role("admin", &["*"]), role("dev", &["dev-read"])];
    }
    config.validate().unwrap();
    config
}

/// Dựng app web; registry có **đúng một** tool `read_file` để toggle được test.
fn build(config: Config) -> (tempfile::TempDir, AxumRouter) {
    set_password(&config.data.dir, PASSWORD).unwrap();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace.clone());
    registry.register(read_file()).unwrap();
    let draft_dir = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft_dir.clone(), draft_dir);
    let router = Arc::new(Router::new(RouterDeps {
        config_path: None,
        default_profile: config.llm.as_profile(),
        config: config.clone(),
        store: store.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(Vec::<LlmResponse>::new())),
        audit: None,
        skills_index: skills.index(),
        skills: Some(skills.clone()),
        llm_state_path: None,
    }));
    let auth = AuthService::load(&config, Arc::clone(&store)).unwrap();
    let state = WebState::new(WebStateDeps {
        config,
        store,
        router,
        auth,
        audit: None,
        skills,
        workspace: Some(workspace),
        speech: None,
    })
    .unwrap();
    (tempfile::tempdir().unwrap(), build_router(state))
}

/// Giống `build` nhưng trả store ra ngoài — test persistence dùng chung store.
fn build_with_store(config: Config) -> (tempfile::TempDir, Arc<dyn Store>, AxumRouter) {
    set_password(&config.data.dir, PASSWORD).unwrap();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace.clone());
    registry.register(read_file()).unwrap();
    let draft_dir = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft_dir.clone(), draft_dir);
    let router = Arc::new(Router::new(RouterDeps {
        config_path: None,
        default_profile: config.llm.as_profile(),
        config: config.clone(),
        store: store.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(Vec::<LlmResponse>::new())),
        audit: None,
        skills_index: skills.index(),
        skills: Some(skills.clone()),
        llm_state_path: None,
    }));
    let auth = AuthService::load(&config, Arc::clone(&store)).unwrap();
    let state = WebState::new(WebStateDeps {
        config,
        store: store.clone(),
        router,
        auth,
        audit: None,
        skills,
        workspace: Some(workspace),
        speech: None,
    })
    .unwrap();
    (tempfile::tempdir().unwrap(), store, build_router(state))
}

fn fixture(viewer: Option<(&str, &str)>) -> (tempfile::TempDir, AxumRouter) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = config_for(&data, &workspace, viewer);
    build(config)
}

async fn login(app: &AxumRouter) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"password":"{PASSWORD}"}}"#)))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap();
    cookie
        .split(';')
        .next()
        .and_then(|value| value.split_once('=').map(|(_, token)| token))
        .unwrap()
        .to_string()
}

async fn toggle(app: &AxumRouter, token: Option<&str>, body: &str) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("PUT")
        .uri("/api/tools/read_file/enabled")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("bean_session={token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn tools_json(app: &AxumRouter, token: &str) -> serde_json::Value {
    let request = Request::builder()
        .method("GET")
        .uri("/api/tools")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn toggle_off_then_on_round_trip_and_persists_in_store() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = config_for(&data, &workspace, None);
    let (_keep, store, app) = build_with_store(config);
    let token = login(&app).await;

    // Mặc định: mọi tool bật.
    let tools = tools_json(&app, &token).await;
    assert_eq!(
        tools["tools"][0]["enabled"],
        serde_json::Value::Bool(true),
        "tool mới phải bật mặc định"
    );

    // Tắt.
    let (status, body) = toggle(&app, Some(&token), r#"{"enabled":false}"#).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["name"], "read_file");
    assert_eq!(parsed["enabled"], false);

    let tools = tools_json(&app, &token).await;
    assert_eq!(tools["tools"][0]["enabled"], false);

    // Store là nguồn sự thật: override 2 chiều, hàng (read_file, enabled = false).
    assert_eq!(
        store.tool_overrides().await.unwrap(),
        vec![ToolOverride {
            name: "read_file".to_string(),
            enabled: false,
        }]
    );

    // Bật lại: override cập nhật đúng hàng (không xoá — phải thắng được seed).
    let (status, body) = toggle(&app, Some(&token), r#"{"enabled":true}"#).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["enabled"], true);
    assert_eq!(
        store.tool_overrides().await.unwrap(),
        vec![ToolOverride {
            name: "read_file".to_string(),
            enabled: true,
        }]
    );
    let tools = tools_json(&app, &token).await;
    assert_eq!(tools["tools"][0]["enabled"], true);
}

#[tokio::test]
async fn toggle_requires_admin_role_when_rbac_enabled() {
    // Role `dev` không phải admin → 403.
    let (_dir, app) = fixture(Some(("web:admin", "dev")));
    let token = login(&app).await;
    let (status, body) = toggle(&app, Some(&token), r#"{"enabled":false}"#).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert!(body.contains("admin_required"), "body: {body}");
}

#[tokio::test]
async fn toggle_admin_role_is_allowed_when_rbac_enabled() {
    let (_dir, app) = fixture(Some(("web:admin", "admin")));
    let token = login(&app).await;
    let (status, body) = toggle(&app, Some(&token), r#"{"enabled":false}"#).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
}

#[tokio::test]
async fn toggle_without_session_is_unauthorized() {
    let (_dir, app) = fixture(None);
    let (status, _) = toggle(&app, None, r#"{"enabled":false}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn toggle_unknown_tool_is_not_found() {
    let (_dir, app) = fixture(None);
    let token = login(&app).await;
    let builder = Request::builder()
        .method("PUT")
        .uri("/api/tools/khong-ton-tai/enabled")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"));
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(r#"{"enabled":false}"#)).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn toggle_survives_restart_semantics_via_shared_store() {
    // Hai WebState dùng chung một store: WebState thứ hai dựng registry mới (mọi tool
    // bật mặc định) nhưng phải thấy tool vẫn tắt vì store còn override.
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace_path = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace_path).unwrap();
    let config = config_for(&data, &workspace_path, None);
    let (_keep, store, app) = build_with_store(config);
    let token = login(&app).await;
    let (status, _) = toggle(&app, Some(&token), r#"{"enabled":false}"#).await;
    assert_eq!(status, StatusCode::OK);

    // "Khởi động lại": app mới, registry mới, cùng store. Override 2 chiều thắng seed.
    let config = config_for(&data, &workspace_path, None);
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace.clone());
    registry.register(read_file()).unwrap();
    for ToolOverride { name, enabled } in store.tool_overrides().await.unwrap() {
        registry.set_tool_enabled(&name, enabled);
    }
    let registry = Arc::new(registry);
    let skills = SkillCatalog::load_with_paths(
        &[],
        config.agent.workspace.join("skills/_drafts"),
        config.agent.workspace.join("skills/_drafts"),
    );
    let router = Arc::new(Router::new(RouterDeps {
        config_path: None,
        default_profile: config.llm.as_profile(),
        config: config.clone(),
        store: store.clone(),
        registry: registry.clone(),
        llm: Arc::new(FakeProvider::new(Vec::<LlmResponse>::new())),
        audit: None,
        skills_index: skills.index(),
        skills: Some(skills.clone()),
        llm_state_path: None,
    }));
    let auth = AuthService::load(&config, Arc::clone(&store)).unwrap();
    let state = WebState::new(WebStateDeps {
        config,
        store: store.clone(),
        router,
        auth,
        audit: None,
        skills,
        workspace: Some(workspace),
        speech: None,
    })
    .unwrap();
    let restarted = build_router(state);

    let request = Request::builder()
        .method("GET")
        .uri("/api/tools")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::empty())
        .unwrap();
    let response = restarted.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let tools: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        tools["tools"][0]["enabled"], false,
        "registry mới phải nạp lại trạng thái tắt từ store: {tools}"
    );
    assert!(registry.is_disabled("read_file"));
}
