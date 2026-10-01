//! Black-box tests M9: HTTP auth/security and WebSocket lifecycle.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use bean_core::{Router, RouterDeps};
use bean_llm::FakeProvider;
use bean_memory::{MemoryStore, SqliteStore, Store};
use bean_security::{AuditLog, CapWorkspace};
use bean_skills::{NewSkillDraft, SkillCatalog, SkillDraftKind};
use bean_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use bean_types::config::{McpServerConfig, RoleConfig};
use bean_types::{Config, LlmResponse, Message, Risk, ToolCall, ToolSpec, Usage};
use bean_web::{
    AuthService, WebState, build_router, set_password, set_password_and_revoke_sessions,
};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

#[derive(Debug)]
struct ConfirmWrite;

#[async_trait]
impl Tool for ConfirmWrite {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "confirm_write",
            "ghi file sau khi người dùng xác nhận",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Confirm
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let path = args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("result.txt");
        let content = args
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("ok");
        ctx.workspace.write_text(path, content)?;
        Ok("written".into())
    }
}

fn make_config(data: &Path, workspace: &Path, origin: &str) -> Config {
    let mut config = Config::default();
    config.data.dir = data.to_path_buf();
    config.agent.workspace = workspace.to_path_buf();
    config.web.public_origin = origin.to_string();
    config.web.session_ttl_hours = 1;
    config.agent.allowed_users = vec!["web:admin".into()];
    config.telegram.enabled = true;
    config.telegram.allowed_user_ids = vec![42];
    config.agent.allowed_users.push("telegram:42".into());
    config
}

async fn make_state(
    config: Config,
    store: Arc<dyn Store>,
    responses: Vec<LlmResponse>,
) -> (AxumRouter, WebState) {
    make_state_with(config, store, responses, Vec::new()).await
}

/// Như [`make_state`] nhưng đăng ký thêm tool — cần cho các test đọc
/// `GET /api/tools` / `/api/mcp`, nơi nội dung payload **là** nội dung registry.
async fn make_state_with(
    config: Config,
    store: Arc<dyn Store>,
    responses: Vec<LlmResponse>,
    extra_tools: Vec<Arc<dyn Tool>>,
) -> (AxumRouter, WebState) {
    set_password(&config.data.dir, PASSWORD).unwrap();
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace.clone());
    registry.register(Arc::new(ConfirmWrite)).unwrap();
    for tool in extra_tools {
        registry.register(tool).unwrap();
    }
    let store_dyn = store;
    let create_root = config.data.dir.join("skills");
    let skills = SkillCatalog::load_with_paths(
        std::slice::from_ref(&create_root),
        create_root.clone(),
        config.agent.workspace.join("skills/_drafts"),
    );
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store_dyn.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(responses)),
        audit: None,
        skills_index: skills.index(),
        skills: Some(skills.clone()),
    }));
    let auth = AuthService::load(&config, store_dyn.clone()).unwrap();
    let state = WebState::new(
        config,
        store_dyn,
        router,
        auth,
        None,
        skills,
        Some(workspace),
    )
    .unwrap();
    (build_router(state.clone()), state)
}

async fn fixture(
    origin: &str,
    responses: Vec<LlmResponse>,
) -> (tempfile::TempDir, AxumRouter, WebState, Arc<MemoryStore>) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = make_config(&data, &workspace, origin);
    let store = Arc::new(MemoryStore::new());
    let (app, state) = make_state(config, store.clone(), responses).await;
    (dir, app, state, store)
}

fn login_request(origin: &str, body: &str, content_type: Option<&str>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(
            header::HOST,
            origin
                .split_once("://")
                .map_or(origin, |(_, authority)| authority),
        )
        .header(header::ORIGIN, origin)
        .header(
            header::CONTENT_TYPE,
            content_type.unwrap_or("application/json"),
        )
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn login(app: &AxumRouter, origin: &str) -> String {
    let response = app
        .clone()
        .oneshot(login_request(
            origin,
            &format!("{{\"password\":\"{PASSWORD}\"}}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap();
    set_cookie
        .split(';')
        .next()
        .and_then(|value| value.split_once('=').map(|(_, token)| token))
        .unwrap()
        .to_string()
}

fn get_request(path: &str, origin: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(path).header(
        header::HOST,
        origin
            .split_once("://")
            .map_or(origin, |(_, authority)| authority),
    );
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("bean_session={token}"));
    }
    builder.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn unauthenticated_and_forbidden_requests_are_json() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;

    let unauthorized = app
        .clone()
        .oneshot(get_request("/api/status", origin, None))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unauthorized.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );

    let mut wrong = login_request(origin, r#"{"password":"x"}"#, None);
    wrong
        .headers_mut()
        .insert(header::ORIGIN, "http://evil.example".parse().unwrap());
    let forbidden = app.oneshot(wrong).await.unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        forbidden.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn authenticated_status_lists_web_and_telegram_channels() {
    use http_body_util::BodyExt;

    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    let token = login(&app, origin).await;
    let response = app
        .oneshot(get_request("/api/status", origin, Some(&token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(status["channels"], serde_json::json!(["web", "telegram"]));
}

#[tokio::test]
async fn security_headers_api_404_and_cookie_flags() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;

    let response = app
        .clone()
        .oneshot(get_request("/api/does-not-exist", origin, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert!(
        response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY)
    );
    assert_eq!(
        response.headers().get("x-content-type-options").unwrap(),
        "nosniff"
    );
    assert_eq!(
        response.headers().get("referrer-policy").unwrap(),
        "no-referrer"
    );
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );

    let login = app
        .clone()
        .oneshot(login_request(
            origin,
            &format!("{{\"password\":\"{PASSWORD}\"}}"),
            None,
        ))
        .await
        .unwrap();
    let cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Path=/"));
    assert!(!cookie.contains("Secure"));
}

#[tokio::test]
async fn mutating_requests_require_json_and_valid_host() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    let valid_body = format!("{{\"password\":\"{PASSWORD}\"}}");

    let mut wrong_host = login_request(origin, &valid_body, None);
    wrong_host
        .headers_mut()
        .insert(header::HOST, "127.0.0.1:9999".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(wrong_host).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    let bad_type = app
        .clone()
        .oneshot(login_request(origin, &valid_body, Some("text/plain")))
        .await
        .unwrap();
    assert_eq!(bad_type.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        bad_type.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );

    let oversized = "x".repeat(bean_web::server::MAX_BODY_BYTES + 1);
    let too_large = app
        .oneshot(login_request(origin, &oversized, None))
        .await
        .unwrap();
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn https_origin_sets_secure_cookie() {
    let origin = "https://agent.example";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    let response = app
        .oneshot(login_request(
            origin,
            &format!("{{\"password\":\"{PASSWORD}\"}}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cookie.contains("Secure"));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
}

#[tokio::test]
async fn auth_audit_never_contains_password_or_token() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = make_config(&data, &workspace, "http://127.0.0.1:7878");
    let store = Arc::new(MemoryStore::new());
    let store_dyn: Arc<dyn Store> = store;
    set_password(&data, PASSWORD).unwrap();
    let auth = AuthService::load(&config, store_dyn).unwrap();
    let audit = AuditLog::open(&data.join("audit")).unwrap();
    let session = auth
        .login(PASSWORD, IpAddr::V4(Ipv4Addr::LOCALHOST), Some(&audit))
        .await
        .unwrap();
    auth.logout(
        &session.token,
        Some(&audit),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
    )
    .await
    .unwrap();
    let content = std::fs::read_to_string(audit.path()).unwrap();
    assert!(content.contains("auth_login"));
    assert!(content.contains("auth_logout"));
    assert!(!content.contains(PASSWORD));
    assert!(!content.contains(&session.token));
}

#[tokio::test]
async fn rest_session_access_is_owner_only() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, store) = fixture(origin, vec![]).await;
    let other = store
        .create_session("web", "web:other", "web:other", "private")
        .await
        .unwrap();
    store.append(other, Message::user("private")).await.unwrap();
    let token = login(&app, origin).await;
    let path = format!("/api/sessions/{}/messages", other.get());
    assert_eq!(
        app.oneshot(get_request(&path, origin, Some(&token)))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn rest_task_crud_validates_cron_and_updates_store() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, store) = fixture(origin, vec![]).await;
    let token = login(&app, origin).await;
    let host = "127.0.0.1:7878";

    let create = Request::builder()
        .method("POST")
        .uri("/api/tasks")
        .header(header::HOST, host)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from(
            r#"{"cron":"0 9 * * *","prompt":"daily summary","channel":"web","chat_id":"web:admin","allowed_tools":[],"enabled":true}"#,
        ))
        .unwrap();
    let response = app.clone().oneshot(create).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let tasks = store.list_tasks().await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].prompt, "daily summary");
    assert!(!tasks[0].next_run.is_empty());

    let toggle = Request::builder()
        .method("PATCH")
        .uri(format!("/api/tasks/{}", tasks[0].id))
        .header(header::HOST, host)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from(r#"{"enabled":false}"#))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(toggle).await.unwrap().status(),
        StatusCode::OK
    );
    assert!(!store.list_tasks().await.unwrap()[0].enabled);

    let invalid = Request::builder()
        .method("POST")
        .uri("/api/tasks")
        .header(header::HOST, host)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from(
            r#"{"cron":"not a cron","prompt":"x","channel":"web","chat_id":"web:admin","allowed_tools":[],"enabled":true}"#,
        ))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(invalid).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    let delete = Request::builder()
        .method("DELETE")
        .uri(format!("/api/tasks/{}", tasks[0].id))
        .header(header::HOST, host)
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(app.oneshot(delete).await.unwrap().status(), StatusCode::OK);
    assert!(store.list_tasks().await.unwrap().is_empty());
}

#[tokio::test]
async fn sqlite_stores_only_hashed_session_tokens_and_rejects_expired_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = make_config(&data, &workspace, "http://127.0.0.1:7878");
    let store = Arc::new(SqliteStore::open(&data.join("bean.db")).unwrap());
    set_password(&data, PASSWORD).unwrap();
    let auth = AuthService::load(&config, store.clone()).unwrap();
    let session = auth
        .login(PASSWORD, IpAddr::V4(Ipv4Addr::LOCALHOST), None)
        .await
        .unwrap();
    let raw = std::fs::read(data.join("bean.db")).unwrap();
    assert!(
        !raw.windows(session.token.len())
            .any(|window| window == session.token.as_bytes())
    );
    assert_eq!(session.token.len(), 64);
    let hash = Sha256::digest(session.token.as_bytes());
    let db = rusqlite::Connection::open(data.join("bean.db")).unwrap();
    let stored: Vec<u8> = db
        .query_row("SELECT token_hash FROM web_sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, hash.as_slice());
    assert_ne!(stored, session.token.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(data.join("auth.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    assert!(auth.authenticate_token(&session.token).await.is_ok());

    let expired = "2000-01-01T00:00:00Z";
    assert!(
        store
            .get_web_session(hash.as_slice(), expired)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .get_web_session(hash.as_slice(), "2999-01-01T00:00:00Z")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn password_rotation_revokes_existing_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let config = make_config(&data, &workspace, "http://127.0.0.1:7878");
    let store = Arc::new(SqliteStore::open(&data.join("bean.db")).unwrap());
    set_password(&data, PASSWORD).unwrap();
    let old = AuthService::load(&config, store.clone())
        .unwrap()
        .login(PASSWORD, IpAddr::V4(Ipv4Addr::LOCALHOST), None)
        .await
        .unwrap();
    set_password_and_revoke_sessions(&data, "new password with enough length", store)
        .await
        .unwrap();
    let updated = AuthService::load(
        &config,
        Arc::new(SqliteStore::open(&data.join("bean.db")).unwrap()),
    )
    .unwrap();
    assert!(updated.authenticate_token(&old.token).await.is_err());
}

#[tokio::test]
async fn logout_and_rate_limit_are_server_side() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    let token = login(&app, origin).await;

    let logout = Request::builder()
        .method("POST")
        .uri("/api/auth/logout")
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(logout).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        app.clone()
            .oneshot(get_request("/api/auth/me", origin, Some(&token)))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(login_request(
                origin,
                r#"{"password":"wrong-password"}"#,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let limited = app
        .oneshot(login_request(
            origin,
            r#"{"password":"wrong-password"}"#,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn websocket_rejects_missing_cookie_and_bad_origin_before_upgrade() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = login(&app, origin).await;
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    let ws_url = format!("ws://{addr}/api/ws");

    let missing_request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(format!("{ws_url}?x=1"))
        .header("Origin", origin)
        .header("Host", addr.to_string())
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .body(())
        .unwrap();
    let missing = connect_async(missing_request).await;
    assert!(missing.is_err(), "WS thiếu cookie phải bị từ chối");

    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&ws_url)
        .header("Origin", "http://evil.example")
        .header("Host", addr.to_string())
        .header("Cookie", format!("bean_session={token}"))
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .body(())
        .unwrap();
    let bad_origin = connect_async(request).await;
    assert!(bad_origin.is_err(), "WS sai Origin phải bị từ chối");
    server.abort();
}

#[tokio::test]
async fn websocket_sync_broadcast_and_confirm_flow_complete() {
    let responses = vec![
        LlmResponse::with_tool_calls(vec![ToolCall::new(
            "call-1",
            "confirm_write",
            serde_json::json!({"path": "from-ws.txt", "content": "ok"}),
        )]),
        LlmResponse::text_only("xong"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace_path = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace_path).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dynamic_origin = format!("http://{addr}");
    let config = make_config(&data, &workspace_path, &dynamic_origin);
    let store = Arc::new(MemoryStore::new());
    let (app, _state) = make_state(config, store.clone(), responses).await;
    let session = store
        .create_session("web", "web:admin", "web:admin", "ws")
        .await
        .unwrap();
    let token = login(&app, &dynamic_origin).await;
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    let origin = dynamic_origin.as_str();
    let url = format!("ws://{addr}/api/ws");
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&url)
        .header("Origin", origin)
        .header("Host", addr.to_string())
        .header("Cookie", format!("bean_session={token}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .body(())
        .unwrap();
    let (mut first, _) = connect_async(request.clone()).await.unwrap();
    let (mut second, _) = connect_async(request).await.unwrap();

    let first_message = tokio::time::timeout(Duration::from_secs(3), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(sync_text) = first_message else {
        panic!("first message phải là text");
    };
    let sync: serde_json::Value = serde_json::from_str(sync_text.as_str()).unwrap();
    assert_eq!(sync["type"], "sync");
    assert!(sync["pending_confirms"].is_array());

    first
        .send(WsMessage::Text(
            serde_json::json!({"type": "start", "session_id": session.get(), "text": "ghi file"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut confirm_id = None;
    for _ in 0..8 {
        let message = tokio::time::timeout(Duration::from_secs(3), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let WsMessage::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        match value["type"].as_str() {
            Some("confirm_request") => {
                confirm_id = Some(value["confirm_id"].as_str().unwrap().to_string());
                break;
            }
            Some("error") => panic!("run lỗi: {value}"),
            _ => {}
        }
    }
    let confirm_id = confirm_id.expect("phải nhận ConfirmRequest");
    let mut second_saw_confirm = false;
    for _ in 0..8 {
        let message = tokio::time::timeout(Duration::from_secs(3), second.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let WsMessage::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        if value["type"] == "confirm_request" {
            assert_eq!(value["confirm_id"], confirm_id);
            second_saw_confirm = true;
            break;
        }
    }
    assert!(second_saw_confirm, "mọi connection phải nhận run event");

    first
        .send(WsMessage::Text(
            serde_json::json!({"type": "confirm", "confirm_id": confirm_id, "decision": "allow"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();

    let mut saw_final = false;
    for _ in 0..12 {
        let message = tokio::time::timeout(Duration::from_secs(3), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let WsMessage::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        if value["type"] == "final" {
            saw_final = true;
            break;
        }
    }
    assert!(saw_final, "phải nhận Final sau confirm");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("workspace/from-ws.txt")).unwrap(),
        "ok"
    );
    server.abort();
}

#[tokio::test]
async fn websocket_reconnect_restores_pending_run_and_survives_disconnect() {
    let responses = vec![
        LlmResponse::with_tool_calls(vec![ToolCall::new(
            "call-reconnect",
            "confirm_write",
            serde_json::json!({"path": "reconnect.txt", "content": "ok"}),
        )]),
        LlmResponse::text_only("xong"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace_path = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace_path).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let config = make_config(&data, &workspace_path, &origin);
    let store = Arc::new(MemoryStore::new());
    let (app, state) = make_state(config, store.clone(), responses).await;
    let session = store
        .create_session("web", "web:admin", "web:admin", "reconnect")
        .await
        .unwrap();
    let token = login(&app, &origin).await;
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    let url = format!("ws://{addr}/api/ws");
    let request = || {
        tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&url)
            .header("Origin", &origin)
            .header("Host", addr.to_string())
            .header("Cookie", format!("bean_session={token}"))
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Sec-WebSocket-Version", "13")
            .body(())
            .unwrap()
    };

    let (mut first, _) = connect_async(request()).await.unwrap();
    let first_sync = tokio::time::timeout(Duration::from_secs(3), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(first_sync) = first_sync else {
        panic!("sync phải là text")
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(first_sync.as_str()).unwrap()["type"],
        "sync"
    );
    first
        .send(WsMessage::Text(
            serde_json::json!({"type": "start", "session_id": session.get(), "text": "ghi file"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut confirm_id = None;
    for _ in 0..8 {
        let message = tokio::time::timeout(Duration::from_secs(3), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let WsMessage::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        if value["type"] == "confirm_request" {
            confirm_id = Some(value["confirm_id"].as_str().unwrap().to_string());
            break;
        }
        assert_ne!(value["type"], "error", "run lỗi: {value}");
    }
    let confirm_id = confirm_id.expect("phải nhận confirm trước disconnect");
    drop(first);

    let (mut reconnected, _) = connect_async(request()).await.unwrap();
    let sync_message = tokio::time::timeout(Duration::from_secs(3), reconnected.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let WsMessage::Text(sync_text) = sync_message else {
        panic!("reconnect phải nhận sync")
    };
    let sync: serde_json::Value = serde_json::from_str(sync_text.as_str()).unwrap();
    assert_eq!(sync["type"], "sync");
    assert!(
        sync["running"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["session_id"] == session.get())
    );
    assert!(
        sync["pending_confirms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["confirm_id"] == confirm_id)
    );
    reconnected
        .send(WsMessage::Text(
            serde_json::json!({"type": "confirm", "confirm_id": confirm_id, "decision": "allow"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut final_seen = false;
    for _ in 0..12 {
        let message = tokio::time::timeout(Duration::from_secs(3), reconnected.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let WsMessage::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        if value["type"] == "confirm_resolved" {
            assert_eq!(value["outcome"], "allowed");
        }
        if value["type"] == "final" {
            final_seen = true;
            break;
        }
    }
    assert!(final_seen, "run phải sống sau khi socket đầu bị đóng");
    assert!(state.router.snapshot().running.is_empty());
    assert_eq!(
        std::fs::read_to_string(workspace_path.join("reconnect.txt")).unwrap(),
        "ok"
    );
    server.abort();
}

#[tokio::test]
async fn skill_drafts_can_be_listed_and_approved_but_are_inactive_first() {
    use http_body_util::BodyExt;

    let origin = "http://127.0.0.1:7878";
    let (_dir, app, state, _store) = fixture(origin, vec![]).await;
    let draft = state
        .skills
        .create_draft(NewSkillDraft {
            name: "release-checklist".into(),
            kind: SkillDraftKind::New,
            description: "Dùng trước khi phát hành.".into(),
            body: "# Steps\n1. Chạy toàn bộ test.".into(),
            reason: "Quy trình này được lặp lại.".into(),
            source_session_id: 1,
            source_channel: "web".into(),
            source_chat_id: "web:admin".into(),
            created_at: "2026-09-25T10:00:00Z".into(),
        })
        .unwrap();
    assert!(state.skills.get("release-checklist").is_err());
    let token = login(&app, origin).await;

    let listed = app
        .clone()
        .oneshot(get_request("/api/skills/drafts", origin, Some(&token)))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body = listed.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["drafts"][0]["id"], draft.id);
    assert_eq!(value["drafts"][0]["kind"], "new");

    let approve = Request::builder()
        .method("POST")
        .uri(format!("/api/skills/drafts/{}/approve", draft.id))
        .header(header::HOST, "127.0.0.1:7878")
        .header(header::ORIGIN, origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("bean_session={token}"))
        .body(Body::from("{}"))
        .unwrap();
    let response = app.clone().oneshot(approve).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.skills.get("release-checklist").is_ok());
    assert!(state.skills.list_drafts().unwrap().is_empty());

    let detail = app
        .oneshot(get_request(
            "/api/skills/release-checklist",
            origin,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
}

// ───────────── Capability endpoints: /tools /mcp /usage /system ─────────────
//
// Bốn endpoint này là nguồn dữ liệu của màn Tools/MCP/Status và widget Hub.
// Test ở đây chốt ba thứ UI không tự kiểm được: RBAC lọc ở server, trạng thái
// MCP suy ra từ registry chứ không bịa, và API không bao giờ lộ secret.

/// Tool có tag RBAC và mức rủi ro cố định — đủ cho test `/api/tools`.
#[derive(Debug)]
struct TaggedTool {
    name: &'static str,
    risk: Risk,
    tags: &'static [&'static str],
    untrusted: bool,
}

#[async_trait]
impl Tool for TaggedTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "tool có tag để test phân quyền",
            serde_json::json!({"type": "object"}),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        self.risk
    }

    fn required_tags(&self) -> Vec<&str> {
        self.tags.to_vec()
    }

    fn marks_untrusted(&self) -> bool {
        self.untrusted
    }

    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        Ok("ok".into())
    }
}

/// Fixture cho phép sửa config và đăng ký thêm tool trước khi dựng app.
async fn fixture_with(
    origin: &str,
    tweak: impl FnOnce(&mut Config),
    extra_tools: Vec<Arc<dyn Tool>>,
) -> (tempfile::TempDir, AxumRouter, Arc<MemoryStore>) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let mut config = make_config(&data, &workspace, origin);
    tweak(&mut config);
    let store = Arc::new(MemoryStore::new());
    let (app, _state) = make_state_with(config, store.clone(), vec![], extra_tools).await;
    (dir, app, store)
}

/// GET một endpoint đã đăng nhập và parse JSON.
async fn get_json(app: &AxumRouter, path: &str, origin: &str, token: &str) -> serde_json::Value {
    use http_body_util::BodyExt;

    let response = app
        .clone()
        .oneshot(get_request(path, origin, Some(token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {path} phải trả 200");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

fn role_config(name: &str, tags: &[&str]) -> RoleConfig {
    RoleConfig {
        name: name.to_string(),
        tool_tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        forbid_tags: Vec::new(),
        allowed_tool_tags: Vec::new(),
        context_budget_tokens: None,
        daily_token_budget: None,
    }
}

#[tokio::test]
async fn capability_endpoints_require_login_and_answer_json() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    for path in ["/api/tools", "/api/mcp", "/api/usage", "/api/system"] {
        let response = app
            .clone()
            .oneshot(get_request(path, origin, None))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "{path} phải cần đăng nhập"
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "{path} phải trả lỗi JSON, không phải HTML"
        );
    }
}

#[tokio::test]
async fn tools_endpoint_reports_risk_source_and_tags() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _store) = fixture_with(
        origin,
        |_| {},
        vec![Arc::new(TaggedTool {
            name: "security_scan",
            risk: Risk::Dangerous,
            tags: &["infra-scan"],
            untrusted: true,
        })],
    )
    .await;
    let token = login(&app, origin).await;
    let body = get_json(&app, "/api/tools", origin, &token).await;

    // RBAC tắt ⇒ mọi tool đều thấy, và UI phải biết điều đó để không hiển thị
    // thông báo "một số tool bị ẩn vì quyền" vô nghĩa.
    assert_eq!(body["rbac_enabled"], false);
    assert_eq!(body["viewer_role"], "default");
    assert_eq!(body["total"], 2);
    let names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["confirm_write", "security_scan"]);

    let scan = &body["tools"][1];
    assert_eq!(scan["risk"], "dangerous");
    assert_eq!(scan["source"], "builtin");
    assert_eq!(scan["mcp_server"], serde_json::Value::Null);
    assert_eq!(scan["required_tags"], serde_json::json!(["infra-scan"]));
    assert_eq!(scan["untrusted"], true);
    assert!(scan["description"].as_str().unwrap().len() > 10);
    assert_eq!(body["tools"][0]["risk"], "confirm");
    assert_eq!(body["tools"][0]["untrusted"], false);
}

#[tokio::test]
async fn tools_endpoint_hides_tagged_tool_from_rbac_role() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _store) = fixture_with(
        origin,
        |config| {
            config.roles = vec![
                role_config("admin", &["*"]),
                role_config("finance-readonly", &["billing-read"]),
            ];
            // RBAC chỉ bật khi user_roles có phần tử; định danh phiên web mặc định
            // là web:admin nên map này đổi luôn role của người đang gọi API.
            config
                .agent
                .user_roles
                .insert("web:admin".to_string(), "finance-readonly".to_string());
        },
        vec![Arc::new(TaggedTool {
            name: "security_scan",
            risk: Risk::Dangerous,
            tags: &["infra-scan"],
            untrusted: true,
        })],
    )
    .await;
    let token = login(&app, origin).await;
    let body = get_json(&app, "/api/tools", origin, &token).await;
    let raw = serde_json::to_string(&body).unwrap();

    // Điều kiện cốt lõi: tool gate-tag không xuất hiện ở BẤT KỲ đâu trong body.
    assert!(
        !raw.contains("security_scan"),
        "tool mang tag infra-scan lọt vào payload của role tài chính: {raw}"
    );
    assert!(raw.contains("confirm_write"), "tool untagged vẫn phải thấy");

    assert_eq!(body["viewer_role"], "finance-readonly");
    assert_eq!(body["rbac_enabled"], true);
    // `total` vẫn là tổng registry: UI nói được "2 tool đang chạy, bạn thấy 1".
    assert_eq!(body["total"], 2);
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn mcp_endpoint_derives_connection_and_never_leaks_env() {
    let origin = "http://127.0.0.1:7878";
    let secret = "super-secret-siem-token";
    let (_dir, app, _store) = fixture_with(
        origin,
        |config| {
            config.mcp_servers.push(McpServerConfig {
                name: "siem".to_string(),
                command: "docker".to_string(),
                args: vec!["run".to_string(), "--rm".to_string()],
                env: BTreeMap::from([("SIEM_TOKEN".to_string(), secret.to_string())]),
                trust: false,
                call_timeout_seconds: Some(300),
                inherit_env: vec![],
                tool_tags: vec!["infra-read".to_string()],
            });
            config.mcp_servers.push(McpServerConfig {
                name: "dead".to_string(),
                command: "/bin/false".to_string(),
                args: vec![],
                env: BTreeMap::new(),
                trust: true,
                call_timeout_seconds: None,
                inherit_env: vec![],
                tool_tags: vec![],
            });
        },
        // Tool duy nhất mang tiền tố `mcp__siem__` ⇒ server `siem` được coi là đã
        // discovery thành công, còn `dead` thì không.
        vec![Arc::new(TaggedTool {
            name: "mcp__siem__cve_lookup",
            risk: Risk::Safe,
            tags: &["infra-read"],
            untrusted: true,
        })],
    )
    .await;
    let token = login(&app, origin).await;
    let body = get_json(&app, "/api/mcp", origin, &token).await;
    let raw = serde_json::to_string(&body).unwrap();

    assert_eq!(body["servers"].as_array().unwrap().len(), 2);
    let siem = &body["servers"][0];
    assert_eq!(siem["name"], "siem");
    assert_eq!(siem["command"], "docker");
    assert_eq!(siem["args"], serde_json::json!(["run", "--rm"]));
    assert_eq!(siem["tool_count"], 1);
    assert_eq!(siem["connected"], true);
    assert_eq!(siem["trusted"], false);
    assert_eq!(siem["required_tags"], serde_json::json!(["infra-read"]));
    assert_eq!(siem["call_timeout_seconds"], serde_json::json!(300));

    let dead = &body["servers"][1];
    assert_eq!(dead["connected"], false);
    assert_eq!(dead["tool_count"], 0);
    assert_eq!(dead["trusted"], true);
    assert_eq!(dead["call_timeout_seconds"], serde_json::Value::Null);

    // `env` của MCP server là secret (mục 15.6): không giá trị, không cả tên biến.
    assert!(
        !raw.contains(secret) && !raw.contains("SIEM_TOKEN") && !raw.contains("\"env\""),
        "biến môi trường của MCP server bị lộ qua API: {raw}"
    );
}

#[tokio::test]
async fn usage_endpoint_is_clamped_chronological_and_never_fabricates() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, store) = fixture(origin, vec![]).await;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    store
        .add_usage(
            &today,
            Usage {
                input_tokens: 1200,
                output_tokens: 300,
            },
        )
        .await
        .unwrap();
    let token = login(&app, origin).await;

    let body = get_json(&app, "/api/usage", origin, &token).await;
    assert_eq!(
        body["days"].as_array().unwrap().len(),
        14,
        "mặc định 14 ngày"
    );
    assert_eq!(body["today_tokens"], 1500);
    assert_eq!(body["daily_token_budget"], 2000000);

    let days = body["days"].as_array().unwrap();
    assert_eq!(days[13]["day"], serde_json::json!(today));
    assert_eq!(days[13]["total_tokens"], 1500);
    assert_eq!(days[13]["input_tokens"], 1200);
    // Ngày không có dữ liệu phải là **0**, không được vắng mặt: biểu đồ thiếu cột
    // thì người dùng đọc là "hệ thống ngừng chạy", sai hoàn toàn.
    assert_eq!(days[12]["total_tokens"], 0);

    for window in days.windows(2) {
        assert!(
            window[0]["day"].as_str().unwrap() < window[1]["day"].as_str().unwrap(),
            "danh sách ngày phải tăng dần: {} rồi {}",
            window[0]["day"],
            window[1]["day"]
        );
    }

    // Trần và sàn: mỗi ngày là một query nên API tự clamp, không đẩy việc đó cho UI.
    let clamped = get_json(&app, "/api/usage?days=999", origin, &token).await;
    assert_eq!(clamped["days"].as_array().unwrap().len(), 31);
    let floored = get_json(&app, "/api/usage?days=0", origin, &token).await;
    assert_eq!(floored["days"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn system_endpoint_reports_config_without_secrets() {
    let origin = "http://127.0.0.1:7878";
    let (_dir, app, _state, _store) = fixture(origin, vec![]).await;
    let token = login(&app, origin).await;
    let body = get_json(&app, "/api/system", origin, &token).await;
    let raw = serde_json::to_string(&body).unwrap();

    assert_eq!(body["sandbox"]["mode"], "docker");
    assert_eq!(body["sandbox"]["network"], false);
    assert_eq!(body["web"]["bind"], "127.0.0.1:7878");
    assert_eq!(body["web"]["allow_remote"], false);
    assert_eq!(body["telegram"]["enabled"], true);
    assert_eq!(body["telegram"]["allowed_users"], 1);
    assert_eq!(body["rbac_enabled"], false);
    assert_eq!(body["projects"], serde_json::json!(["default"]));
    assert_eq!(body["provider"], "anthropic");
    assert_eq!(body["api_key_env"], "ANTHROPIC_API_KEY");
    assert_eq!(body["max_steps"], 25);

    // Những thứ phải **không** có mặt: mật khẩu, token phiên đang dùng, bí mật Telegram.
    assert!(
        !raw.contains(PASSWORD),
        "mật khẩu đăng nhập không được xuất hiện ở bất kỳ endpoint nào"
    );
    assert!(
        !raw.contains(&token),
        "token phiên không được echo lại qua API"
    );
    assert!(
        !raw.contains("TELEGRAM_BOT_TOKEN") && !raw.contains("token_env"),
        "tên biến chứa bot token không thuộc về response: {raw}"
    );
    // Tên biến API key thì được phép: đó là cách cấu hình, không phải bí mật.
    assert!(raw.contains("ANTHROPIC_API_KEY"));
}
