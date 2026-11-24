//! Black-box tests cho `POST /api/system/restart` và phần `restart` của
//! `GET /api/status` (M31 — nút "Khởi động lại ngay" kiểu Ubuntu).
//!
//! # Trọng tâm
//!
//! * 401 khi chưa đăng nhập, 403 cho user không phải admin (restart ảnh hưởng
//!   khả dụng nhưng không lộ secret — admin-only đủ, không cần step-up);
//! * 409 `run_active` khi còn run đang chạy — restart giữa run sẽ để lại cặp
//!   tool_use/tool_result hỏng;
//! * 409 `restart_pending` khi một restart đã được yêu cầu mà chưa hoàn tất;
//! * sai `Origin` → 403 (guard CSRF chuẩn của tầng `/api`);
//! * happy path 202 + handle restart nhận đúng 1 lần gọi;
//! * `GET /api/status` tổng hợp đủ lý do chờ restart (khoá cấu hình lệch đĩa
//!   và marker `bean update`), và rỗng khi sạch.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use bean_core::{Router, RouterDeps};
use bean_llm::FakeProvider;
use bean_memory::MemoryStore;
use bean_security::CapWorkspace;
use bean_skills::SkillCatalog;
use bean_tools::ToolRegistry;
use bean_tools::builtin::files::tool::{read_file, write_file};
use bean_types::config::RoleConfig;
use bean_types::{Config, LlmResponse, ToolCall};
use bean_web::{
    AuthService, NoopRebind, ProcessRestart, WebState, WebStateDeps, build_router, set_password,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";
const ORIGIN: &str = "http://127.0.0.1:7878";

/// Fake handle restart kiểu recorder: lần đầu nhận yêu cầu (`true`), mọi lần sau
/// trả `false` — mô phỏng "một restart đã được yêu cầu và chưa hoàn tất".
#[derive(Default)]
struct FakeRestart(AtomicBool);

impl FakeRestart {
    fn calls(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl ProcessRestart for FakeRestart {
    fn request(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }
}

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

/// `bean.toml` trên đĩa với `[agent].workspace` khác giá trị đang chạy — khoá
/// `apply = restart` thật (`agent.workspace`, tier B) để test lý do "cấu hình
/// đã ghi đĩa nhưng chưa hiệu lực".
fn write_config_file(path: &Path, workspace: &Path) {
    std::fs::write(
        path,
        format!("[agent]\nworkspace = \"{}\"\n", workspace.display()),
    )
    .unwrap();
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: AxumRouter,
    router: Arc<Router>,
    restart: Arc<FakeRestart>,
    data_dir: PathBuf,
}

fn build(
    viewer: Option<(&str, &str)>,
    with_config_file: bool,
    responses: Vec<LlmResponse>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config_file = dir.path().join("bean.toml");
    if with_config_file {
        // Đĩa nói workspace khác RAM → khoá `agent.workspace` rơi vào "chờ restart".
        write_config_file(&config_file, Path::new("/elsewhere-on-disk"));
    }
    let config = config_for(&data, &workspace, viewer);

    set_password(&config.data.dir, PASSWORD, None).unwrap();
    let store: Arc<dyn bean_memory::Store> = Arc::new(MemoryStore::new());
    let cap_workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(cap_workspace.clone());
    registry.register(read_file()).unwrap();
    registry.register(write_file()).unwrap();
    let draft_dir = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft_dir.clone(), draft_dir);
    let router = Arc::new(Router::new(RouterDeps {
        config_path: with_config_file.then(|| config_file.clone()),
        default_profile: config.llm.as_profile(),
        config: config.clone(),
        store: store.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(responses)),
        audit: None,
        skills_index: skills.index(),
        skills: Some(skills.clone()),
        llm_state_path: None,
        speech: None,
        checkpoints: None,
    }));
    let auth = AuthService::load(&config, Arc::clone(&store)).unwrap();
    let restart = Arc::new(FakeRestart::default());
    let state = WebState::new(WebStateDeps {
        config,
        store,
        router: router.clone(),
        auth,
        audit: None,
        skills,
        workspace: Some(cap_workspace),
        speech: None,
        rebind: Arc::new(NoopRebind),
        restart: restart.clone(),
        os_auth: Arc::new(bean_security::FakeOsAuth::default()),
        sandbox_mode: sandbox_mode_cell(),
    })
    .unwrap();
    Fixture {
        _dir: dir,
        app: build_router(state),
        router,
        restart,
        data_dir: data,
    }
}

fn sandbox_mode_cell() -> Arc<tokio::sync::watch::Sender<bean_types::config::SandboxMode>> {
    let (tx, _rx) = tokio::sync::watch::channel(bean_types::config::SandboxMode::Docker);
    Arc::new(tx)
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

/// POST `/api/system/restart` — `origin` tuỳ chọn để test guard CSRF.
async fn post_restart(
    app: &AxumRouter,
    token: Option<&str>,
    origin: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/system/restart")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, origin.unwrap_or(ORIGIN));
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("bean_session={token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from("{}")).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn get_status(app: &AxumRouter, token: &str) -> serde_json::Value {
    let request = Request::builder()
        .method("GET")
        .uri("/api/status")
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

/// Đợi tới khi `snapshot().running` không rỗng — run đã mark active ngay trong
/// `submit` nhưng poll cho chắc, tránh race với spawn.
async fn wait_running(router: &Router) {
    for _ in 0..200 {
        if !router.snapshot().running.is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("run chưa vào snapshot.running sau 5s");
}

#[tokio::test]
async fn restart_requires_login() {
    let fx = build(None, false, Vec::new());
    let (status, body) = post_restart(&fx.app, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body}");
    assert!(
        !fx.restart.calls(),
        "handle không được gọi khi chưa đăng nhập"
    );
}

#[tokio::test]
async fn restart_requires_admin() {
    let fx = build(Some(("web:dev", "dev")), false, Vec::new());
    let token = login(&fx.app).await;
    let (status, body) = post_restart(&fx.app, Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert!(!fx.restart.calls());
}

#[tokio::test]
async fn restart_rejects_wrong_origin() {
    let fx = build(None, false, Vec::new());
    let token = login(&fx.app).await;
    let (status, body) = post_restart(&fx.app, Some(&token), Some("http://evil.example")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert!(!fx.restart.calls());
}

#[tokio::test]
async fn restart_blocked_while_run_active() {
    // Run chờ confirm: FakeProvider trả tool call `write_file` (Confirm) — run
    // treo ở ConfirmRequest, `snapshot().running` không rỗng.
    let responses = vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
        "c1",
        "write_file",
        serde_json::json!({"path": "x.txt", "content": "hi"}),
    )])];
    let fx = build(Some(("web:admin", "admin")), false, responses);
    let token = login(&fx.app).await;
    fx.router
        .submit(bean_core::Incoming::new(
            "web",
            "chat-1",
            "web:admin",
            "ghi file hộ tôi",
        ))
        .await
        .unwrap();
    wait_running(&fx.router).await;

    let (status, body) = post_restart(&fx.app, Some(&token), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert!(body.contains("run_active"), "body: {body}");
    assert!(!fx.restart.calls(), "handle không được gọi khi còn run");
}

#[tokio::test]
async fn restart_happy_path_202_then_pending_conflict() {
    let fx = build(Some(("web:admin", "admin")), false, Vec::new());
    let token = login(&fx.app).await;
    let (status, body) = post_restart(&fx.app, Some(&token), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["restarting"], serde_json::Value::Bool(true));
    assert!(fx.restart.calls(), "handle phải nhận đúng một lần gọi");

    // Fake đã consumed → lần sau phải 409 restart_pending, không gọi handle thêm.
    let (status, body) = post_restart(&fx.app, Some(&token), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
    assert!(body.contains("restart_pending"), "body: {body}");
}

#[tokio::test]
async fn status_is_clean_when_nothing_pending() {
    let fx = build(Some(("web:admin", "admin")), false, Vec::new());
    let token = login(&fx.app).await;
    let status = get_status(&fx.app, &token).await;
    assert_eq!(
        status["restart"]["required"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(
        status["restart"]["reasons"],
        serde_json::Value::Array(vec![])
    );
}

#[tokio::test]
async fn status_reports_marker_and_pending_config_keys() {
    let fx = build(Some(("web:admin", "admin")), true, Vec::new());
    bean_update::write_pending_marker(&fx.data_dir, "v1.2.3").unwrap();

    let token = login(&fx.app).await;
    let status = get_status(&fx.app, &token).await;
    assert_eq!(status["restart"]["required"], serde_json::Value::Bool(true));
    let reasons = status["restart"]["reasons"].as_array().unwrap();
    assert_eq!(
        reasons.len(),
        2,
        "cả marker update và khoá cấu hình lệch đĩa phải được tổng hợp: {reasons:?}"
    );
    let joined = reasons
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(joined.contains("v1.2.3"), "reasons: {joined}");
    assert!(joined.contains("agent.workspace"), "reasons: {joined}");
}
