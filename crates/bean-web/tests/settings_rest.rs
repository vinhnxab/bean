//! Black-box tests cho REST cài đặt thống nhất của M29: `/api/settings`,
//! `/api/secrets` và `/api/settings/test/{target}`.
//!
//! # Trọng tâm bảo mật
//!
//! * Chưa đăng nhập → 401; RBAC bật + role thường → 403 cho **mọi** mutation.
//! * Tầng B (bảo mật) chỉ sửa được qua CLI — web trả `403 cli_only` kèm đúng lệnh.
//! * Ghi thiết lập bắt buộc audit và audit **không chứa giá trị**.
//! * PUT/DELETE secret yêu cầu step-up mật khẩu, dùng chung rate-limiter với login:
//!   5 lần sai → khoá, thử đúng mật khẩu vẫn 429 (session đánh cắp không dò được).
//! * Payload secret **không bao giờ** có trường giá trị; tên ngoài slot bị từ chối.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use bean_core::{Router, RouterDeps, RouterOptions};
use bean_llm::FakeProvider;
use bean_memory::{MemoryStore, Store};
use bean_security::{AuditLog, CapWorkspace};
use bean_skills::SkillCatalog;
use bean_tools::{ToolRegistry, builtin::files::tool::read_file};
use bean_types::config::RoleConfig;
use bean_types::{Config, LlmResponse};
use bean_web::{AuthService, WebState, WebStateDeps, build_router, set_password};
use http_body_util::BodyExt;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";
const ORIGIN: &str = "http://127.0.0.1:7878";
/// `bean.toml` tối thiểu cho test — `SettingsService` tự load/validate khi PATCH.
const BEAN_TOML: &str = "[agent]\nworkspace = \"./workspace\"\n";
/// Tên slot không bao giờ trùng biến môi trường thật của máy chạy test.
const SLOT: &str = "BEAN_TEST_REST_KEY";

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
    // Slot riêng cho test: kể cả máy có ANTHROPIC_API_KEY trong env thì kết quả
    // `GET /api/secrets` của test vẫn xác định.
    config.llm.api_key_env = SLOT.to_string();
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

/// Dựng app web với kho secret và (tuỳ chọn) `bean.toml` thật cho Router.
fn build(config: Config, config_toml: Option<&Path>) -> AxumRouter {
    set_password(&config.data.dir, PASSWORD, None).unwrap();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace.clone());
    registry.register(read_file()).unwrap();
    let draft_dir = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft_dir.clone(), draft_dir);
    let secret_store = Arc::new(bean_settings::SecretStore::new(
        config.data.dir.join("secrets.env"),
    ));
    let router = Arc::new(Router::with_options(
        RouterDeps {
            config_path: config_toml.map(Path::to_path_buf),
            default_profile: config.llm.as_profile(),
            config: config.clone(),
            store: store.clone(),
            registry: Arc::new(registry),
            llm: Arc::new(FakeProvider::new(Vec::<LlmResponse>::new())),
            audit: None,
            skills_index: skills.index(),
            skills: Some(skills.clone()),
            llm_state_path: None,
            speech: None,
        },
        RouterOptions {
            secrets: Some(secret_store),
            ..RouterOptions::default()
        },
    ));
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
    build_router(state)
}

/// Fixture chuẩn: có `bean.toml` (PATCH ghi được) + data dir + workspace riêng.
fn fixture(viewer: Option<(&str, &str)>) -> (tempfile::TempDir, PathBuf, AxumRouter) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config_toml = dir.path().join("bean.toml");
    std::fs::write(&config_toml, BEAN_TOML).unwrap();
    let config = config_for(&data, &workspace, viewer);
    let app = build(config, Some(&config_toml));
    (dir, config_toml, app)
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

async fn request_json(
    app: &AxumRouter,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("bean_session={token}"));
    }
    let response = app
        .clone()
        .oneshot(
            builder
                .body(Body::from(body.unwrap_or("{}").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

fn code_of(json: &serde_json::Value) -> String {
    json.get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ────────────────── /api/settings ──────────────────

#[tokio::test]
async fn settings_requires_login() {
    let (_dir, _toml, app) = fixture(None);
    let (status, _) = request_json(&app, "GET", "/api/settings", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn settings_list_shape_and_tier_flags() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("writable"), Some(&serde_json::Value::Bool(true)));
    let settings = json
        .get("settings")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert!(!settings.is_empty());
    let find = |key: &str| {
        settings
            .iter()
            .find(|entry| entry.get("key").and_then(serde_json::Value::as_str) == Some(key))
            .unwrap()
            .clone()
    };
    // Tầng B: chỉ xem, UI phải khoá form.
    let sandbox_mode = find("security.sandbox.mode");
    assert_eq!(
        sandbox_mode.get("tier").and_then(serde_json::Value::as_str),
        Some("B")
    );
    assert_eq!(
        sandbox_mode.get("editable_in_ui"),
        Some(&serde_json::Value::Bool(false))
    );
    // Tầng A: sửa được từ UI.
    let timezone = find("agent.timezone");
    assert_eq!(
        timezone.get("tier").and_then(serde_json::Value::as_str),
        Some("A")
    );
    assert_eq!(
        timezone.get("editable_in_ui"),
        Some(&serde_json::Value::Bool(true))
    );
    // Đủ trường cho UI dựng form + mô tả song ngữ.
    for field in [
        "key", "value", "source", "tier", "apply", "kind", "desc_vi", "desc_en",
    ] {
        assert!(
            timezone
                .get(field)
                .is_some_and(serde_json::Value::is_string),
            "thiếu trường `{field}`"
        );
    }
}

#[tokio::test]
async fn patch_tier_b_is_rejected_as_cli_only() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"security.sandbox.mode","value":"host"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code_of(&json), "cli_only");
    // Gợi ý phải là lệnh CLI thay thế, không phải thông báo chung chung.
    let message = json
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    assert!(message.contains("bean config set security.sandbox.mode"));
}

#[tokio::test]
async fn patch_tier_a_writes_toml_and_audits_without_value() {
    let (dir, config_toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"agent.timezone","value":"Asia/Ho_Chi_Minh"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("source").and_then(serde_json::Value::as_str),
        Some("bean.toml")
    );
    assert_eq!(
        json.get("restart_required"),
        Some(&serde_json::Value::Bool(false))
    );
    // File cấu hình thật được sửa (toml_edit giữ định dạng).
    let written = std::fs::read_to_string(&config_toml).unwrap();
    assert!(written.contains("timezone = \"Asia/Ho_Chi_Minh\""));

    // Audit phải ghi, nhưng không bao giờ chứa giá trị.
    let data = dir.path().join("data");
    let audit = AuditLog::open(&data.join("audit")).unwrap();
    let entries = audit.read_recent(None, 100);
    assert!(
        entries.iter().any(
            |entry| entry.get("tool").and_then(serde_json::Value::as_str) == Some("config.set")
        ),
        "thiếu dòng audit config.set: {entries:?}"
    );
    let raw = serde_json::to_string(&entries).unwrap();
    assert!(
        !raw.contains("Asia/Ho_Chi_Minh"),
        "audit không được chứa giá trị: {raw}"
    );
}

#[tokio::test]
async fn patch_unknown_key_hints_secret_flow() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    // Tên ALL_CAPS → hướng sang `bean secret set` như CLI.
    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"ANTHROPIC_API_KEY","value":"x"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&json), "unknown_setting");
    let message = json
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap();
    assert!(message.contains("bean secret set"));

    // Khoá lạ thường → gợi ý `bean config list`.
    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"totally.made_up","value":"x"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&json), "unknown_setting");
    assert!(
        json.get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap()
            .contains("bean config list")
    );
}

#[tokio::test]
async fn patch_bad_value_kind_is_rejected() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"agent.max_steps","value":"abc"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&json), "invalid_value");
}

#[tokio::test]
async fn patch_without_config_path_is_readonly() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = config_for(&data, &workspace, None);
    let app = build(config, None);
    let token = login(&app).await;

    let (_, json) = request_json(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(json.get("writable"), Some(&serde_json::Value::Bool(false)));

    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"agent.timezone","value":"UTC"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(code_of(&json), "config_readonly");
}

// ────────────────── /api/secrets ──────────────────

#[tokio::test]
async fn secrets_list_has_no_value_field() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(&app, "GET", "/api/secrets", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let slots = json
        .get("slots")
        .and_then(serde_json::Value::as_array)
        .unwrap();
    assert!(!slots.is_empty());
    for slot in slots {
        let keys: Vec<&str> = slot
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(
            !keys.contains(&"value"),
            "slot không được có field giá trị: {keys:?}"
        );
    }
    let slot = slots
        .iter()
        .find(|slot| slot.get("name").and_then(serde_json::Value::as_str) == Some(SLOT))
        .unwrap();
    assert_eq!(slot.get("set"), Some(&serde_json::Value::Bool(false)));
    assert_eq!(
        slot.get("source").and_then(serde_json::Value::as_str),
        Some("unset")
    );
    assert_eq!(
        slot.get("group").and_then(serde_json::Value::as_str),
        Some("llm")
    );
}

#[tokio::test]
async fn secret_put_roundtrip_file_mode_and_list() {
    let (dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "PUT",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(r#"{"value":"sk-test-123","password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("ok"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(
        json.get("source").and_then(serde_json::Value::as_str),
        Some("secrets_env")
    );

    // File secrets.env: tồn tại, quyền 0600, có dòng `NAME=value`.
    let path = dir.path().join("data").join("secrets.env");
    let metadata = std::fs::metadata(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains(&format!("{SLOT}=sk-test-123")));

    // GET phải phản ánh trạng thái mới — vẫn không có giá trị.
    let (_, json) = request_json(&app, "GET", "/api/secrets", Some(&token), None).await;
    let slot = json
        .get("slots")
        .and_then(serde_json::Value::as_array)
        .unwrap()
        .iter()
        .find(|slot| slot.get("name").and_then(serde_json::Value::as_str) == Some(SLOT))
        .unwrap();
    assert_eq!(slot.get("set"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(
        slot.get("source").and_then(serde_json::Value::as_str),
        Some("secrets_env")
    );
}

#[tokio::test]
async fn secret_put_rejects_unknown_slot() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "PUT",
        "/api/secrets/PATH",
        Some(&token),
        Some(r#"{"value":"/evil","password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&json), "unknown_slot");
}

#[tokio::test]
async fn secret_put_requires_password_and_locks_after_failures() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    // 5 lần sai mật khẩu → 401 từng lần (lần thứ 5 đặt khoá 1 giây).
    for attempt in 0..5 {
        let (status, json) = request_json(
            &app,
            "PUT",
            &format!("/api/secrets/{SLOT}"),
            Some(&token),
            Some(r#"{"value":"sk-x","password":"wrong password entirely"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "lần {attempt}");
        assert_eq!(code_of(&json), "invalid_password");
    }
    // Lần thứ 6 → rate limiter dùng chung với login khoá IP.
    let (status, json) = request_json(
        &app,
        "PUT",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(r#"{"value":"sk-x","password":"wrong password entirely"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(code_of(&json), "rate_limited");

    // Khoá lần đầu chỉ 1 giây; hết khoá (cửa sổ 60 giây còn, bộ đếm giữ nguyên) thì
    // sai tiếp vẫn 401 và khoá tăng dần lên 2 giây.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let (status, json) = request_json(
        &app,
        "PUT",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(r#"{"value":"sk-x","password":"wrong password entirely"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(code_of(&json), "invalid_password");

    // ĐÚNG mật khẩu cũng bị khoá — dò qua step-up không rẻ hơn dò qua login.
    let (status, _) = request_json(
        &app,
        "PUT",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(r#"{"value":"sk-x","password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn secret_delete_roundtrip() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let password = format!(r#"{{"password":"{PASSWORD}"}}"#);
    let (status, _) = request_json(
        &app,
        "PUT",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(r#"{"value":"sk-del","password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, json) = request_json(
        &app,
        "DELETE",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(&password),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("ok"), Some(&serde_json::Value::Bool(true)));

    let (_, json) = request_json(&app, "GET", "/api/secrets", Some(&token), None).await;
    let slot = json
        .get("slots")
        .and_then(serde_json::Value::as_array)
        .unwrap()
        .iter()
        .find(|slot| slot.get("name").and_then(serde_json::Value::as_str) == Some(SLOT))
        .unwrap();
    assert_eq!(slot.get("set"), Some(&serde_json::Value::Bool(false)));
}

// ────────────────── RBAC ──────────────────

#[tokio::test]
async fn rbac_non_admin_can_read_but_not_mutate() {
    let (_dir, _toml, app) = fixture(Some(("dev", "dev")));
    let token = login(&app).await;

    // GET đọc được (không có giá trị secret nên không rò rỉ gì).
    let (status, _) = request_json(&app, "GET", "/api/settings", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request_json(&app, "GET", "/api/secrets", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);

    // Mutation: 403 admin_required cho cả settings lẫn secret.
    let (status, json) = request_json(
        &app,
        "PATCH",
        "/api/settings",
        Some(&token),
        Some(r#"{"key":"agent.timezone","value":"UTC"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code_of(&json), "admin_required");

    let (status, json) = request_json(
        &app,
        "PUT",
        &format!("/api/secrets/{SLOT}"),
        Some(&token),
        Some(r#"{"value":"sk-x","password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code_of(&json), "admin_required");
}

// ────────────────── /api/settings/test/{target} ──────────────────

#[tokio::test]
async fn settings_test_llm_reports_error_without_secrets() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    // Provider mặc định là anthropic — cần key mà slot test chưa có ⇒ lỗi rõ ràng,
    // không đặt mạng thật, không lộ gì ngoài tên biến.
    let (status, json) =
        request_json(&app, "POST", "/api/settings/test/llm", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("target").and_then(serde_json::Value::as_str),
        Some("llm")
    );
    assert_eq!(
        json.get("level").and_then(serde_json::Value::as_str),
        Some("error")
    );
    let raw = serde_json::to_string(&json).unwrap();
    assert!(raw.contains("không resolve được secret LLM"));
    assert!(raw.contains("bean secret set"));
    assert!(
        !raw.contains("sk-"),
        "response không được chứa giá trị key: {raw}"
    );
}

#[tokio::test]
async fn settings_test_voice_local_warns() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    // Provider mặc định `local` — không có gì kiểm qua mạng, trả warn kèm hướng dẫn.
    let (status, json) =
        request_json(&app, "POST", "/api/settings/test/voice", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("level").and_then(serde_json::Value::as_str),
        Some("warn")
    );
    assert!(
        json.get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap()
            .contains("local")
    );
}

#[tokio::test]
async fn settings_test_unknown_target_is_422() {
    let (_dir, _toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "POST",
        "/api/settings/test/nuclear",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&json), "unknown_target");
}

#[tokio::test]
async fn settings_test_requires_admin_under_rbac() {
    let (_dir, _toml, app) = fixture(Some(("dev", "dev")));
    let token = login(&app).await;
    let (status, json) =
        request_json(&app, "POST", "/api/settings/test/llm", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code_of(&json), "admin_required");
}

// ────────────────── /api/settings/lan-access (D21.x) ──────────────────

const BEAN_TOML_LAN: &str = concat!(
    "[agent]\nworkspace = \"./workspace\"\n\n",
    "[web]\nbind = \"0.0.0.0:7878\"\nallow_remote = true\n",
    "public_origin = \"http://192.168.1.29:7878\"\n"
);

/// Cùng thuật toán dò IP LAN với handler — để test quyết định kỳ vọng theo máy
/// chạy (có đường ra ⇒ bật LAN được; không ⇒ server phải trả 422 fail-closed).
fn probe_lan_ipv4() -> Option<std::net::Ipv4Addr> {
    for probe in ["8.8.8.8:80", "1.1.1.1:443"] {
        let Ok(socket) = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0)) else {
            continue;
        };
        if socket.connect(probe).is_err() {
            continue;
        }
        let Ok(local) = socket.local_addr() else {
            continue;
        };
        if let std::net::IpAddr::V4(ip) = local.ip()
            && !ip.is_loopback()
        {
            return Some(ip);
        }
    }
    None
}

#[tokio::test]
async fn lan_access_requires_login() {
    let (_dir, _toml, app) = fixture(None);
    let (status, _) = request_json(&app, "GET", "/api/settings/lan-access", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        None,
        Some(r#"{"enabled":true,"password":"x"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn lan_access_requires_admin_under_rbac() {
    let (_dir, _toml, app) = fixture(Some(("dev", "dev")));
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        Some(&token),
        Some(r#"{"enabled":false,"password":"irrelevant"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(code_of(&json), "admin_required");
    // GET vẫn được — chỉ đọc.
    let (status, _) =
        request_json(&app, "GET", "/api/settings/lan-access", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn lan_access_rejects_wrong_password() {
    let (_dir, config_toml, app) = fixture(None);
    std::fs::write(&config_toml, BEAN_TOML_LAN).unwrap();
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        Some(&token),
        Some(r#"{"enabled":false,"password":"wrong password entirely"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(code_of(&json), "invalid_password");
    // File không được đụng.
    let written = std::fs::read_to_string(&config_toml).unwrap();
    assert!(
        written.contains("0.0.0.0:7878"),
        "file phải còn nguyên: {written}"
    );
}

#[tokio::test]
async fn lan_access_disable_writes_loopback_atomically_and_audits() {
    let (dir, config_toml, app) = fixture(None);
    std::fs::write(&config_toml, BEAN_TOML_LAN).unwrap();
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        Some(&token),
        Some(r#"{"enabled":false,"password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("restart_required"),
        Some(&serde_json::Value::Bool(true))
    );
    let access = json.get("access").unwrap();
    assert_eq!(
        access.get("mode").and_then(serde_json::Value::as_str),
        Some("loopback")
    );
    assert_eq!(access.get("port"), Some(&serde_json::json!(7878)));
    assert_eq!(access.get("url"), Some(&serde_json::Value::Null));

    // Cả ba khoá phải đổi cùng lúc trong file.
    let written = std::fs::read_to_string(&config_toml).unwrap();
    assert!(
        written.contains("127.0.0.1:7878"),
        "bind phải về loopback: {written}"
    );
    assert!(
        written.contains("allow_remote = false"),
        "phải tắt remote: {written}"
    );
    assert!(
        written.contains("http://127.0.0.1:7878"),
        "public_origin phải về loopback: {written}"
    );

    // Audit: một event `config.set` cho mỗi khoá, không chứa giá trị.
    let data = dir.path().join("data");
    let audit = AuditLog::open(&data.join("audit")).unwrap();
    let entries = audit.read_recent(None, 100);
    let config_sets = entries
        .iter()
        .filter(|entry| entry.get("tool").and_then(serde_json::Value::as_str) == Some("config.set"))
        .count();
    assert_eq!(config_sets, 3, "mỗi khoá một event: {entries:?}");
    let raw = serde_json::to_string(&entries).unwrap();
    assert!(
        !raw.contains("192.168"),
        "audit không được chứa giá trị: {raw}"
    );
}

#[tokio::test]
async fn lan_access_get_reflects_pending_state_before_and_after_apply() {
    // Lỗi thật 2026-11-08: GET đọc snapshot in-memory lúc khởi động nên sau khi
    // POST ghi file thành công, UI refetch GET thấy trạng thái cũ và công tắc
    // bật lại vị trí cũ. GET phải đọc bean.toml trên đĩa (nguồn sự thật chờ
    // restart, như GET /api/settings).
    let (_dir, config_toml, app) = fixture(None);
    std::fs::write(&config_toml, BEAN_TOML_LAN).unwrap();
    let token = login(&app).await;

    // Trước Apply: file khai LAN ⇒ GET phải thấy "lan" (chờ restart), dù tiến
    // trình đang chạy vẫn bind theo snapshot cũ.
    let (status, json) =
        request_json(&app, "GET", "/api/settings/lan-access", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("mode").and_then(serde_json::Value::as_str),
        Some("lan"),
        "GET phải phản ánh bean.toml trên đĩa: {json}"
    );

    // Sau Apply tắt LAN: GET phải thấy "loopback" (trạng thái ghi vào đĩa).
    let (status, _) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        Some(&token),
        Some(r#"{"enabled":false,"password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, json) =
        request_json(&app, "GET", "/api/settings/lan-access", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json.get("mode").and_then(serde_json::Value::as_str),
        Some("loopback"),
        "sau Apply, GET phải thấy trạng thái chờ restart: {json}"
    );
    assert_eq!(json.get("url"), Some(&serde_json::Value::Null));
}

#[tokio::test]
async fn lan_access_enable_uses_detected_lan_or_fails_closed() {
    let (_dir, config_toml, app) = fixture(None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        Some(&token),
        Some(r#"{"enabled":true,"password":"correct horse battery staple"}"#),
    )
    .await;
    let written = std::fs::read_to_string(&config_toml).unwrap();
    match probe_lan_ipv4() {
        None => {
            // Không có đường ra — server phải từ chối bật (fail-closed), không ghi.
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(code_of(&json), "lan_ip_unavailable");
            assert!(
                !written.contains("0.0.0.0"),
                "không được ghi bind LAN khi không dò được IP: {written}"
            );
        }
        Some(ip) => {
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json.get("access")
                    .and_then(|a| a.get("mode"))
                    .and_then(serde_json::Value::as_str),
                Some("lan")
            );
            assert!(
                written.contains("0.0.0.0:7878"),
                "bind phải mở LAN: {written}"
            );
            assert!(
                written.contains("allow_remote = true"),
                "phải bật remote: {written}"
            );
            let expected = format!("http://{ip}:7878");
            assert!(
                written.contains(&expected),
                "public_origin phải theo IP LAN {ip}: {written}"
            );
        }
    }
}

#[tokio::test]
async fn lan_access_is_readonly_without_config_path() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = config_for(&data, &workspace, None);
    let app = build(config, None);
    let token = login(&app).await;
    let (status, json) = request_json(
        &app,
        "POST",
        "/api/settings/lan-access",
        Some(&token),
        Some(r#"{"enabled":true,"password":"correct horse battery staple"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(code_of(&json), "config_readonly");
}
