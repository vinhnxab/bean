//! Black-box tests M27: REST voice (status / transcribe / synthesize) qua axum thật.
//!
//! # Bằng chứng
//!
//! * Voice tắt ⇒ `503 voice_disabled` (fail-closed), không âm thầm trả kết quả rỗng.
//! * Chưa đăng nhập ⇒ `401` trước khi chạm provider.
//! * Audio hỏng (không phải base64) ⇒ `400`, không panic.
//! * STT/TTS dùng `FakeSpeech` — không mạng, không whisper/piper thật.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::Router as AxumRouter;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bean_core::{Router, RouterDeps};
use bean_llm::FakeProvider;
use bean_memory::{MemoryStore, Store};
use bean_security::CapWorkspace;
use bean_skills::SkillCatalog;
use bean_speech::{FakeSpeech, SpeechProvider};
use bean_tools::ToolRegistry;
use bean_types::{Config, LlmResponse};
use bean_web::{AuthService, WebState, WebStateDeps, build_router, set_password};
use http_body_util::BodyExt;
use tower::ServiceExt;

const ORIGIN: &str = "http://127.0.0.1:7878";
const HOST: &str = "127.0.0.1:7878";
const PASSWORD: &str = "correct horse battery staple";
/// base64 của `"fake-audio"` (byte thật không quan trọng — [`FakeSpeech`] bỏ qua).
const AUDIO_B64: &str = "ZmFrZS1hdWRpbw==";

/// Dựng app với voice bật/tắt; `None` mô phỏng `[voice].enabled = false`.
async fn fixture(speech_enabled: bool) -> (tempfile::TempDir, AxumRouter) {
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
    // Mirror serve.rs: voice khai báo đủ + seed bật khi fixture dựng provider;
    // `None` = `[voice]` chưa cấu hình (không có model/key nào để dựng).
    config.voice.enabled = speech_enabled;
    // Tên biến env riêng cho test: nếu dùng mặc định `ELEVENLABS_API_KEY` thì máy dev
    // tình cờ có biến đó sẽ làm test "api thiếu key → 422" thất bại không xác định.
    config.voice.api_key_env = "BEAN_TEST_ELEVENLABS_KEY".to_string();

    set_password(&config.data.dir, PASSWORD).unwrap();
    let workspace_fs = Arc::new(CapWorkspace::open(config.agent.workspace.clone()).unwrap());
    let registry = ToolRegistry::with_workspace(workspace_fs.clone());
    let draft = config.agent.workspace.join("skills/_drafts");
    let skills = SkillCatalog::load_with_paths(&[], draft.clone(), draft);
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
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
    let auth = AuthService::load(&config, store.clone()).unwrap();
    let speech: Option<Arc<dyn SpeechProvider>> = if speech_enabled {
        Some(Arc::new(FakeSpeech::new("xin chào")))
    } else {
        None
    };
    let state = WebState::new(WebStateDeps {
        config,
        store,
        router,
        auth,
        audit: None,
        skills,
        workspace: Some(workspace_fs),
        speech,
    })
    .unwrap();
    (dir, build_router(state))
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

fn patch_json(path: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method("PATCH")
        .uri(path)
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("bean_session={token}"));
    }
    builder.body(Body::from(body.to_owned())).unwrap()
}

async fn login(app: &AxumRouter) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!("{{\"password\":\"{PASSWORD}\"}}")))
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

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn voice_status_requires_auth_then_reports_enabled() {
    let (_dir, app) = fixture(true).await;

    let unauthorized = app
        .clone()
        .oneshot(get_request("/api/voice", None))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let token = login(&app).await;
    let response = app
        .clone()
        .oneshot(get_request("/api/voice", Some(&token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_string(response).await;
    assert!(payload.contains("\"enabled\":true"), "{payload}");
    assert!(payload.contains("\"language\":\"vi\""), "{payload}");
    assert!(payload.contains("\"provider\":\"local\""), "{payload}");
}

#[tokio::test]
async fn transcribe_returns_fake_transcript() {
    let (_dir, app) = fixture(true).await;
    let token = login(&app).await;
    let body = format!(r#"{{"audio_base64":"{AUDIO_B64}","mime":"audio/webm"}}"#);
    let response = app
        .clone()
        .oneshot(post_json("/api/voice/transcribe", &token, &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_string(response).await;
    assert!(payload.contains("xin chào"), "{payload}");
}

#[tokio::test]
async fn synthesize_returns_base64_audio() {
    let (_dir, app) = fixture(true).await;
    let token = login(&app).await;
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/voice/synthesize",
            &token,
            r#"{"text":"xin chào"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_string(response).await;
    assert!(payload.contains("\"audio_base64\""), "{payload}");
    assert!(payload.contains("audio/mpeg"), "{payload}");
}

#[tokio::test]
async fn voice_disabled_is_fail_closed() {
    let (_dir, app) = fixture(false).await;
    let token = login(&app).await;

    let status = app
        .clone()
        .oneshot(get_request("/api/voice", Some(&token)))
        .await
        .unwrap();
    let payload = body_string(status).await;
    assert!(payload.contains("\"enabled\":false"), "{payload}");

    let response = app
        .clone()
        .oneshot(post_json(
            "/api/voice/synthesize",
            &token,
            r#"{"text":"xin chào"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let payload = body_string(response).await;
    assert!(payload.contains("voice_disabled"), "{payload}");
}

#[tokio::test]
async fn transcribe_rejects_invalid_base64_without_panic() {
    let (_dir, app) = fixture(true).await;
    let token = login(&app).await;
    let response = app
        .clone()
        .oneshot(post_json(
            "/api/voice/transcribe",
            &token,
            r#"{"audio_base64":"!!!not base64!!!","mime":"audio/webm"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let payload = body_string(response).await;
    assert!(payload.contains("invalid_audio"), "{payload}");
}

#[tokio::test]
async fn voice_update_requires_auth() {
    let (_dir, app) = fixture(true).await;
    let response = app
        .clone()
        .oneshot(patch_json("/api/voice", None, r#"{"enabled":false}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// Toggle từ UI: tắt ⇒ GET enabled:false và STT/TTS fail-closed; bật lại ⇒ dùng được
// ngay. File `voice-state.json` phải được ghi để trạng thái sống sót qua restart.
#[tokio::test]
async fn voice_toggle_persists_and_gates_stt_tts() {
    let (dir, app) = fixture(true).await;
    let token = login(&app).await;

    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":false}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_string(response).await;
    assert!(payload.contains("\"enabled\":false"), "{payload}");
    assert!(payload.contains("\"configured\":true"), "{payload}");

    let status = app
        .clone()
        .oneshot(get_request("/api/voice", Some(&token)))
        .await
        .unwrap();
    assert!(body_string(status).await.contains("\"enabled\":false"));

    for path in ["/api/voice/transcribe", "/api/voice/synthesize"] {
        let body = if path.ends_with("transcribe") {
            format!(r#"{{"audio_base64":"{AUDIO_B64}","mime":"audio/webm"}}"#)
        } else {
            r#"{"text":"xin chào"}"#.to_string()
        };
        let response = app
            .clone()
            .oneshot(post_json(path, &token, &body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        let payload = body_string(response).await;
        assert!(payload.contains("voice_disabled"), "{payload}");
    }

    let state_file = dir.path().join("data").join("voice-state.json");
    let saved = std::fs::read_to_string(&state_file).unwrap();
    assert!(saved.contains("\"enabled\":false"), "{saved}");

    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":true}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let transcript = app
        .clone()
        .oneshot(post_json(
            "/api/voice/transcribe",
            &token,
            &format!(r#"{{"audio_base64":"{AUDIO_B64}","mime":"audio/webm"}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(transcript.status(), StatusCode::OK);
    assert!(body_string(transcript).await.contains("xin chào"));
}

#[tokio::test]
async fn voice_update_rejected_when_not_configured() {
    let (_dir, app) = fixture(false).await;
    let token = login(&app).await;
    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":true}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = body_string(response).await;
    assert!(payload.contains("voice_not_configured"), "{payload}");
}

// Đổi sang provider `api` kèm key từ UI: dựng được ElevenLabs ngay lúc chạy (không cần
// restart), persist provider/language/key vào state file 0600, và response/GET **không
// bao giờ** trả key nguyên vẹn về client.
#[tokio::test]
async fn voice_update_to_api_persists_key_and_never_echoes_it() {
    let (dir, app) = fixture(true).await;
    let token = login(&app).await;

    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":true,"provider":"api","api_key":"sk-test-1234abcd"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = body_string(response).await;
    assert!(payload.contains("\"provider\":\"api\""), "{payload}");
    assert!(payload.contains("\"api_key_set\":true"), "{payload}");
    assert!(!payload.contains("sk-test-1234abcd"), "rò key: {payload}");

    let state_file = dir.path().join("data").join("voice-state.json");
    let saved = std::fs::read_to_string(&state_file).unwrap();
    assert!(saved.contains("\"provider\":\"api\""), "{saved}");
    assert!(saved.contains("\"language\":\"vi\""), "{saved}");
    assert!(saved.contains("sk-test-1234abcd"), "{saved}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&state_file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "file chứa key phải là 0600");
    }

    // GET sau khi đổi: provider/api + api_key_set, nhưng vẫn không lộ key.
    let status = app
        .clone()
        .oneshot(get_request("/api/voice", Some(&token)))
        .await
        .unwrap();
    let payload = body_string(status).await;
    assert!(payload.contains("\"provider\":\"api\""), "{payload}");
    assert!(payload.contains("\"api_key_set\":true"), "{payload}");
    assert!(payload.contains("\"enabled\":true"), "{payload}");
    assert!(!payload.contains("sk-test-1234abcd"), "rò key: {payload}");
}

// Chọn `api` mà không có key (không nhập, không env) ⇒ từ chối 422 và KHÔNG persist —
// state trên đĩa không được rơi vào trạng thái nửa vời.
#[tokio::test]
async fn voice_update_api_without_key_is_rejected_and_not_persisted() {
    let (dir, app) = fixture(true).await;
    let token = login(&app).await;

    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":true,"provider":"api"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let payload = body_string(response).await;
    assert!(payload.contains("voice_not_configured"), "{payload}");

    let state_file = dir.path().join("data").join("voice-state.json");
    assert!(!state_file.exists(), "không được persist khi dựng thất bại");
}

#[tokio::test]
async fn voice_update_rejects_unknown_provider_and_empty_language() {
    let (_dir, app) = fixture(true).await;
    let token = login(&app).await;

    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":true,"provider":"cloud"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body_string(response).await.contains("invalid_provider"));

    let response = app
        .clone()
        .oneshot(patch_json(
            "/api/voice",
            Some(&token),
            r#"{"enabled":true,"language":"   "}"#,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body_string(response).await.contains("invalid_language"));
}

#[tokio::test]
async fn transcribe_accepts_body_over_global_limit() {
    // WAV 16 kHz mono 16-bit ≈ 32 KB/giây; 1,2 MB thô ≈ 1,6 MB sau base64 — vượt
    // trần chung `MAX_BODY_BYTES` (1 MB) nhưng phải nằm trong trần riêng của route
    // (`VOICE_TRANSCRIBE_BODY_BYTES`) thay vì bị chặn 413 (lỗi thật khi UI ghi WAV).
    let (_dir, app) = fixture(true).await;
    let token = login(&app).await;
    let audio_b64 = BASE64.encode(vec![0u8; 1_200_000]);
    let response = app
        .oneshot(post_json(
            "/api/voice/transcribe",
            &token,
            &format!(r#"{{"audio_base64":"{audio_b64}","mime":"audio/wav"}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("xin chào"));
}
