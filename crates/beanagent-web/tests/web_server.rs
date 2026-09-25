//! Black-box tests M9: HTTP auth/security and WebSocket lifecycle.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use beanagent_core::{Router, RouterDeps};
use beanagent_llm::FakeProvider;
use beanagent_memory::{MemoryStore, SqliteStore, Store};
use beanagent_security::{AuditLog, CapWorkspace};
use beanagent_skills::SkillCatalog;
use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use beanagent_types::{Config, LlmResponse, Message, Risk, ToolCall, ToolSpec};
use beanagent_web::{
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
    config
}

async fn make_state(
    config: Config,
    store: Arc<dyn Store>,
    responses: Vec<LlmResponse>,
) -> (AxumRouter, WebState) {
    set_password(&config.data.dir, PASSWORD).unwrap();
    let workspace = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace.clone());
    registry.register(Arc::new(ConfirmWrite)).unwrap();
    let store_dyn = store;
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store_dyn.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(responses)),
        audit: None,
        skills_index: String::new(),
    }));
    let auth = AuthService::load(&config, store_dyn.clone()).unwrap();
    let state = WebState::new(
        config,
        store_dyn,
        router,
        auth,
        None,
        SkillCatalog::load(&[]),
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
        builder = builder.header(header::COOKIE, format!("beanagent_session={token}"));
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

    let oversized = "x".repeat(beanagent_web::server::MAX_BODY_BYTES + 1);
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
        .header(header::COOKIE, format!("beanagent_session={token}"))
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
        .header(header::COOKIE, format!("beanagent_session={token}"))
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
        .header(header::COOKIE, format!("beanagent_session={token}"))
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
        .header(header::COOKIE, format!("beanagent_session={token}"))
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
    let store = Arc::new(SqliteStore::open(&data.join("beanagent.db")).unwrap());
    set_password(&data, PASSWORD).unwrap();
    let auth = AuthService::load(&config, store.clone()).unwrap();
    let session = auth
        .login(PASSWORD, IpAddr::V4(Ipv4Addr::LOCALHOST), None)
        .await
        .unwrap();
    let raw = std::fs::read(data.join("beanagent.db")).unwrap();
    assert!(
        !raw.windows(session.token.len())
            .any(|window| window == session.token.as_bytes())
    );
    assert_eq!(session.token.len(), 64);
    let hash = Sha256::digest(session.token.as_bytes());
    let db = rusqlite::Connection::open(data.join("beanagent.db")).unwrap();
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
    let store = Arc::new(SqliteStore::open(&data.join("beanagent.db")).unwrap());
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
        Arc::new(SqliteStore::open(&data.join("beanagent.db")).unwrap()),
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
        .header(header::COOKIE, format!("beanagent_session={token}"))
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
        .header("Cookie", format!("beanagent_session={token}"))
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
        .header("Cookie", format!("beanagent_session={token}"))
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
            .header("Cookie", format!("beanagent_session={token}"))
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
