//! Black-box tests cho `GET /api/agents` — nguồn dữ liệu của HUB.
//!
//! # Trọng tâm: RBAC phải lọc ở TẦNG API, không ở client
//!
//! Yêu cầu bắt buộc là role `finance-readonly` **không được nhận** dữ liệu agent
//! ngoài `billing-read`. "Ẩn bằng CSS" không phải phân quyền: payload vẫn còn trong
//! response và ai cũng đọc được bằng DevTools hay `curl`.
//!
//! Vì vậy các test ở đây khẳng định trên **chuỗi JSON thật** của response: tên của
//! agent bị chặn không được xuất hiện ở bất kỳ đâu trong body — kể cả khi UI có
//! render nó.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use beanagent_core::{Router, RouterDeps};
use beanagent_llm::FakeProvider;
use beanagent_memory::{MemoryStore, Store};
use beanagent_security::CapWorkspace;
use beanagent_skills::SkillCatalog;
use beanagent_tools::ToolRegistry;
use beanagent_types::config::RoleConfig;
use beanagent_types::{Config, LlmResponse};
use beanagent_web::{AuthService, WebState, build_router, set_password};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";
const ORIGIN: &str = "http://127.0.0.1:7878";

/// Tên role dùng để kiểm tra `relation`; test đổi tên sẽ dùng bản thay thế.
const QA: &str = "qa";
const SECURITY: &str = "security-scan";

fn role(name: &str, tags: &[&str], forbid: &[&str]) -> RoleConfig {
    RoleConfig {
        name: name.to_string(),
        tool_tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        forbid_tags: forbid.iter().map(|tag| (*tag).to_string()).collect(),
        allowed_tool_tags: Vec::new(),
        context_budget_tokens: None,
        daily_token_budget: None,
    }
}

/// Hệ agent như `BeanAgent.example.toml`: admin, review four-eyes, quét bảo mật
/// và tài chính đọc-only.
fn roles() -> Vec<RoleConfig> {
    vec![
        role("admin", &["*"], &[]),
        role("developer", &["dev-write"], &[]),
        // four-eyes: được review nhưng không bao giờ được sửa.
        role(QA, &["dev-read", "test-run"], &["dev-write"]),
        role(SECURITY, &["infra-read", "infra-scan"], &[]),
        role("finance-readonly", &["billing-read"], &[]),
    ]
}

fn config_for(data: &Path, workspace: &Path, viewer: Option<(&str, &str)>) -> Config {
    let mut config = Config::default();
    config.data.dir = data.to_path_buf();
    config.agent.workspace = workspace.to_path_buf();
    config.web.public_origin = ORIGIN.to_string();
    config.web.session_ttl_hours = 1;
    config.roles = roles();
    if let Some((user, role_name)) = viewer {
        // RBAC chỉ bật khi `user_roles` có phần tử.
        config
            .agent
            .user_roles
            .insert(user.to_string(), role_name.to_string());
        config.agent.allowed_users = vec![user.to_string()];
        // Định danh phiên web: nếu không đặt, mọi phiên đều là `web:admin` và role
        // của viewer dưới đây sẽ không bao giờ được áp dụng.
        config.web.user_id = user.to_string();
    }
    config.validate().unwrap();
    config
}

/// Dựng app web từ config đã validate; giữ thư mục tạm sống suốt test.
fn build(config: Config) -> (tempfile::TempDir, AxumRouter) {
    set_password(&config.data.dir, PASSWORD).unwrap();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let registry = ToolRegistry::with_workspace(workspace.clone());
    let draft_dir = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft_dir.clone(), draft_dir);
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(Vec::<LlmResponse>::new())),
        audit: None,
        skills_index: skills.index(),
        skills: Some(skills.clone()),
    }));
    let auth = AuthService::load(&config, Arc::clone(&store)).unwrap();
    let state = WebState::new(config, store, router, auth, None, skills, Some(workspace)).unwrap();
    (tempfile::tempdir().unwrap(), build_router(state))
}

/// Fixture chuẩn: thư mục tạm + app đã dựng.
fn fixture(viewer: Option<(&str, &str)>) -> (tempfile::TempDir, AxumRouter) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = config_for(&data, &workspace, viewer);
    let (_keep, app) = build(config);
    (dir, app)
}

/// Fixture có thể sửa cấu hình trước khi dựng app.
fn fixture_raw(tweak: impl FnOnce(&mut Config)) -> (tempfile::TempDir, PathBuf, PathBuf, Config) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = config_for(&data, &workspace, Some(("web:admin", "admin")));
    tweak(&mut config);
    config.validate().unwrap();
    (dir, data, workspace, config)
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

async fn get_agents(app: &AxumRouter, token: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("GET")
        .uri("/api/agents")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN);
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("beanagent_session={token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Quan hệ của một role, đọc từ chính chuỗi JSON trả về.
fn relation_of(body: &str, role: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    parsed["agents"]
        .as_array()
        .expect("agents phải là mảng")
        .iter()
        .find(|agent| agent["role"] == role)
        .unwrap_or_else(|| panic!("không thấy agent `{role}` trong: {body}"))["relation"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn agents_endpoint_requires_login() {
    let (_dir, app) = fixture(Some(("web:admin", "admin")));
    let (status, _body) = get_agents(&app, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Test bắt buộc: finance-readonly KHÔNG được nhận dữ liệu agent khác
// ---------------------------------------------------------------------------

#[tokio::test]
async fn finance_readonly_response_excludes_every_other_agent() {
    let (_dir, app) = fixture(Some(("web:finance", "finance-readonly")));
    let token = login(&app).await;
    let (status, body) = get_agents(&app, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);

    // Điều kiện cốt lõi: agent bị chặn không xuất hiện trong payload. Không chỉ
    // thiếu ở `agents[]` — tên nó không được xuất hiện ở BẤT KỲ đâu trong body,
    // vì lọc sót ở một mảng con vẫn là rò dữ liệu.
    for hidden in ["developer", "qa", "security-scan", "admin"] {
        assert!(
            !body.contains(hidden),
            "response chứa `{hidden}` — dữ liệu agent ngoài billing-read đã lọt ra API"
        );
    }
    // Domain của chính nó thì phải có, nếu không HUB của role tài chính sẽ trống rỗng
    // vô ích và người dùng không hiểu vì sao.
    assert!(
        body.contains("finance-readonly"),
        "phải thấy domain của chính mình"
    );
    assert!(body.contains("\"viewer_role\":\"finance-readonly\""));
}

#[tokio::test]
async fn finance_readonly_gets_exactly_its_own_domain() {
    let (_dir, app) = fixture(Some(("web:finance", "finance-readonly")));
    let token = login(&app).await;
    let (status, body) = get_agents(&app, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let agents = parsed["agents"].as_array().unwrap();
    assert_eq!(
        agents.len(),
        1,
        "role tài chính chỉ được thấy đúng 1 agent, thấy: {body}"
    );
    assert_eq!(agents[0]["role"], "finance-readonly");
    // Báo cáo chuẩn hoá: đủ ba trường, không lộ dữ liệu thô.
    assert_eq!(agents[0]["status"], "idle");
    assert!(agents[0]["summary"].is_string());
    assert_eq!(agents[0]["risks"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn admin_sees_every_agent_with_its_own_status() {
    let (_dir, app) = fixture(Some(("web:admin", "admin")));
    let token = login(&app).await;
    let (status, body) = get_agents(&app, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let agents = parsed["agents"].as_array().unwrap();
    let mut roles: Vec<&str> = agents
        .iter()
        .map(|agent| agent["role"].as_str().unwrap())
        .collect();
    roles.sort_unstable();
    assert_eq!(
        roles,
        [
            "admin",
            "developer",
            "finance-readonly",
            "qa",
            "security-scan"
        ]
    );
    // Mọi agent rảnh khi chưa có run nào — trạng thái phải suy ra từ tín hiệu thật,
    // không phải giá trị mặc định bịa ra.
    for agent in agents {
        assert_eq!(agent["status"], "idle", "chưa có run thì phải rảnh");
        assert!(agent["summary"].is_string());
    }
}

// ---------------------------------------------------------------------------
// Quan hệ kiến trúc: sơ đồ phải phản ánh cấu hình, không hardcode tên role
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relations_come_from_configured_tags() {
    let (_dir, app) = fixture(Some(("web:admin", "admin")));
    let token = login(&app).await;
    let (_status, body) = get_agents(&app, Some(&token)).await;

    // four-eyes: `qa` có tag đọc nhưng không có tag ghi ⇒ nét đôi review.
    assert_eq!(relation_of(&body, QA), "reviews");
    // D14.11: role quét có `infra-scan` ⇒ kênh cảnh báo thẳng tới người quản trị.
    assert_eq!(relation_of(&body, SECURITY), "alerts_directly");
    // Còn lại do Manager điều phối.
    assert_eq!(relation_of(&body, "developer"), "manages");
    assert_eq!(relation_of(&body, "finance-readonly"), "manages");
}

#[tokio::test]
async fn renaming_a_role_does_not_change_its_architecture() {
    // Sơ đồ được vẽ từ **tag**, không từ tên role: đổi tên không được làm UI vẽ sai
    // kiến trúc — kiểu hỏng âm thầm nguy hiểm nhất vì không ai báo lỗi.
    let (_dir, _data, _workspace, mut config) = fixture_raw(|_| {});
    for entry in &mut config.roles {
        if entry.name == QA {
            entry.name = "reviewer".into();
        }
        if entry.name == SECURITY {
            entry.name = "quan-tri-bao-mat".into();
        }
    }
    config.validate().unwrap();
    let (_keep, app) = build(config);

    let token = login(&app).await;
    let (_status, body) = get_agents(&app, Some(&token)).await;
    assert_eq!(relation_of(&body, "reviewer"), "reviews");
    assert_eq!(relation_of(&body, "quan-tri-bao-mat"), "alerts_directly");
}

#[tokio::test]
async fn no_access_role_is_denoed_every_agent() {
    // User không được map vào role nào ⇒ resolve ra `no-access` (deny-all, D11.1).
    // HUB phải rỗng, không lọt agent nào — kể cả khi danh sách trắng rỗng.
    let (_dir, _data, _workspace, config) = fixture_raw(|config| {
        // KHÔNG thêm vào `user_roles` ⇒ `permissions_for` trả `no-access`.
        config.agent.allowed_users.push("web:stranger".into());
        config.web.user_id = "web:stranger".into();
    });
    let (_keep, app) = build(config);

    let token = login(&app).await;
    let (status, body) = get_agents(&app, Some(&token)).await;
    // `validate()` chặn role không tồn tại nên `no-access` đến từ user **không được
    // map**; dù server trả 200 hay 403 thì body tuyệt đối không được chứa tên agent.
    assert!(
        status == StatusCode::OK || status == StatusCode::FORBIDDEN,
        "trạng thái lạ: {status} — body: {body}"
    );
    if status == StatusCode::OK {
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            parsed["agents"].as_array().unwrap().len(),
            0,
            "no-access phải thấy 0 agent, thấy: {body}"
        );
    }
    for hidden in ["finance-readonly", "security-scan", "developer"] {
        assert!(!body.contains(hidden), "`{hidden}` lọt cho no-access");
    }
}
