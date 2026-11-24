//! Black-box tests Web Push REST (status / key / subscribe / unsubscribe / test).
//!
//! # Bằng chứng
//!
//! * Chưa đăng nhập ⇒ `401` trước khi chạm khóa VAPID.
//! * `GET /api/push/key` trả public key 65 byte (uncompressed, `0x04`) — lần sau
//!   gọi vẫn là cùng khóa (persist ở `data.dir/push-vapid.json`).
//! * `POST /api/push/subscribe` lưu subscription; GET `/api/push` liệt kê với
//!   `label` (hash, không lộ URL) và `failure_count = 0`; đăng ký lại cập nhật,
//!   không nhân đôi.
//! * Endpoint `http` / khóa sai định dạng ⇒ `422` (fail-closed, không lưu).
//! * `POST /api/push/test` với không subscription ⇒ `{delivered: 0}` — không mạng.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use bean_core::{Router, RouterDeps};
use bean_llm::FakeProvider;
use bean_memory::{MemoryStore, Store};
use bean_security::CapWorkspace;
use bean_skills::SkillCatalog;
use bean_tools::ToolRegistry;
use bean_types::{Config, LlmResponse};
use bean_web::{AuthService, NoopRebind, WebState, WebStateDeps, build_router, set_password};
use http_body_util::BodyExt;
use tower::ServiceExt;

const ORIGIN: &str = "http://127.0.0.1:7878";
const HOST: &str = "127.0.0.1:7878";
const PASSWORD: &str = "correct horse battery staple";
/// Public key P-256 uncompressed hợp lệ (vector RFC 8291) — chỉ dùng để vượt
/// validation, không gọi mạng nào với nó trong test này.
const VALID_P256DH: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const VALID_AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
const ENDPOINT: &str = "https://fcm.googleapis.com/fcm/send/test-token-abc";

async fn fixture() -> (tempfile::TempDir, AxumRouter) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    let mut config = Config::default();
    config.data.dir = data.clone();
    config.agent.workspace = workspace.clone();
    config.web.public_origin = ORIGIN.to_string();
    config.web.session_ttl_hours = 1;
    config.agent.allowed_users = vec!["web:admin".into()];
    config.voice.enabled = false;

    set_password(&config.data.dir, PASSWORD, None).unwrap();
    let workspace_fs = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let registry = ToolRegistry::with_workspace(workspace_fs.clone());
    let draft = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft.clone(), draft);
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let router = Arc::new(Router::with_options(
        RouterDeps {
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
            speech: None,
            checkpoints: None,
        },
        bean_core::RouterOptions::default(),
    ));
    let auth = AuthService::load(&config, store.clone()).unwrap();
    let state = WebState::new(WebStateDeps {
        config,
        store,
        router,
        auth,
        audit: None,
        skills,
        workspace: Some(workspace_fs),
        speech: None,
        rebind: Arc::new(NoopRebind),
        os_auth: Arc::new(bean_security::FakeOsAuth::default()),
        sandbox_mode: sandbox_mode_cell(),
    })
    .unwrap();
    (dir, build_router(state))
}

fn sandbox_mode_cell() -> Arc<tokio::sync::watch::Sender<bean_types::config::SandboxMode>> {
    let (sender, _receiver) = tokio::sync::watch::channel(bean_types::config::SandboxMode::Docker);
    Arc::new(sender)
}

fn get_request(path: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(path)
        .header(header::HOST, HOST);
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("bean_session={token}"));
    }
    builder.body(Body::empty()).unwrap()
}

fn post_json(path: &str, token: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn login(app: &AxumRouter) -> String {
    let request = post_json_bodyless(
        "/api/auth/login",
        &format!("{{\"password\":\"{PASSWORD}\"}}"),
    );
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

fn post_json_bodyless(path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn subscribe_body(endpoint: &str, p256dh: &str, auth: &str) -> String {
    format!(
        "{{\"endpoint\":\"{endpoint}\",\"keys\":{{\"p256dh\":\"{p256dh}\",\"auth\":\"{auth}\"}}}}"
    )
}

#[tokio::test]
async fn push_key_requires_auth() {
    let (_dir, app) = fixture().await;
    let response = app
        .clone()
        .oneshot(get_request("/api/push/key", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app.oneshot(get_request("/api/push", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Public key ổn định giữa các lần gọi — file khóa persist trong data.dir.
#[tokio::test]
async fn push_key_returns_stable_public_key() {
    let (_dir, app) = fixture().await;
    let token = login(&app).await;
    let response = app
        .clone()
        .oneshot(get_request("/api/push/key", Some(&token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(
        body.contains("\"public_key\":\""),
        "thiếu public_key: {body}"
    );

    let response = app
        .clone()
        .oneshot(get_request("/api/push/key", Some(&token)))
        .await
        .unwrap();
    let body2 = body_string(response).await;
    assert_eq!(body, body2, "public_key phải ổn định giữa hai lần gọi");

    // Khóa phải là 65 byte base64url bắt đầu 0x04 (uncompressed P-256).
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    let key = value["public_key"].as_str().unwrap();
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(key)
        .unwrap();
    assert_eq!(raw.len(), 65);
    assert_eq!(raw[0], 0x04);
}

/// Chu kỳ đầy đủ: subscribe → liệt kê → đăng ký lại (không nhân đôi) → gỡ.
#[tokio::test]
async fn subscribe_list_resubscribe_unsubscribe() {
    let (_dir, app) = fixture().await;
    let token = login(&app).await;

    let response = app
        .clone()
        .oneshot(post_json(
            "/api/push/subscribe",
            &token,
            &subscribe_body(ENDPOINT, VALID_P256DH, VALID_AUTH),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .clone()
        .oneshot(get_request("/api/push", Some(&token)))
        .await
        .unwrap();
    let body = body_string(response).await;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["configured"], true);
    assert_eq!(value["subscriptions"].as_array().unwrap().len(), 1);
    assert!(
        value["subscriptions"][0]["label"]
            .as_str()
            .unwrap()
            .starts_with("push-")
    );
    assert_eq!(value["subscriptions"][0]["failure_count"], 0);

    // Đăng ký lại cùng endpoint: UPSERT — vẫn 1 bản ghi.
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/push/subscribe",
            &token,
            &subscribe_body(ENDPOINT, VALID_P256DH, VALID_AUTH),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = app
        .clone()
        .oneshot(get_request("/api/push", Some(&token)))
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
    assert_eq!(value["subscriptions"].as_array().unwrap().len(), 1);

    // Gỡ: 204, lần nữa: 404.
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/push/unsubscribe",
            &token,
            &format!("{{\"endpoint\":\"{ENDPOINT}\"}}"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/push/unsubscribe",
            &token,
            &format!("{{\"endpoint\":\"{ENDPOINT}\"}}"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Endpoint không phải https bị từ chối — dữ liệu client không tin cậy.
#[tokio::test]
async fn subscribe_rejects_http_endpoint() {
    let (_dir, app) = fixture().await;
    let token = login(&app).await;
    let response = app
        .oneshot(post_json(
            "/api/push/subscribe",
            &token,
            &subscribe_body(
                "http://fcm.googleapis.com/fcm/send/x",
                VALID_P256DH,
                VALID_AUTH,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// Khóa sai định dạng bị từ chối trước khi lưu.
#[tokio::test]
async fn subscribe_rejects_invalid_keys() {
    let (_dir, app) = fixture().await;
    let token = login(&app).await;
    let response = app
        .oneshot(post_json(
            "/api/push/subscribe",
            &token,
            &subscribe_body(ENDPOINT, "khong-phai-key", VALID_AUTH),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// `POST /api/push/test` không subscription: 200 với delivered = 0 (không mạng).
#[tokio::test]
async fn push_test_without_subscriptions() {
    let (_dir, app) = fixture().await;
    let token = login(&app).await;
    let response = app
        .clone()
        .oneshot(post_json("/api/push/test", &token, "{}"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["delivered"], 0);
    assert_eq!(value["failed"], 0);
    assert_eq!(value["removed"], 0);
}

/// Guard CSRF: thiếu `Origin` cho request thay đổi dữ liệu ⇒ 403 (mục 15.7).
#[tokio::test]
async fn subscribe_requires_origin_header() {
    let (_dir, app) = fixture().await;
    let token = login(&app).await;
    let request = Request::builder()
        .method("POST")
        .uri("/api/push/subscribe")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from(subscribe_body(
            ENDPOINT,
            VALID_P256DH,
            VALID_AUTH,
        )))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
