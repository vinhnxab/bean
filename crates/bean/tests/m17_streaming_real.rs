//! Kiểm chứng streaming thật (M17) xuyên qua binary `serve` + WebSocket — opt-in,
//! KHÔNG chạy trong `make check`.
//!
//! Chạy tay:
//! `set -a; . ~/.bean/secrets.env; set +a; cargo test -p bean --test m17_streaming_real -- --ignored --nocapture`
//!
//! Luồng: binary thật → provider NVIDIA (SSE) → agent loop → Router → WS `text_delta`.
//! Khẳng định: lượt chạy trả về **nhiều hơn 2** delta text không rỗng trước `Final`
//! (nền cho chữ hiện dần trong UI) và text ghép lại khớp nội dung yêu cầu.
//! UI hiển thị delta đã được phủ bởi unit test (`web/src/store/chat.test.ts`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Stdio;
use std::time::Duration;

use bean_types::Config;
use bean_types::config::LlmProviderKind;
use bean_web::{ServerMsg, set_password};
use futures_util::{SinkExt, StreamExt};
use reqwest::header::{CONTENT_TYPE, ORIGIN, SET_COOKIE};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message as WsMessage, http::Request},
};

const PASSWORD: &str = "m17-real-password-123456";
const BASE_URL: &str = "https://integrate.api.nvidia.com/v1";
const MODEL: &str = "z-ai/glm-5.3-flash";
const KEY_ENV: &str = "NVIDIA_KEY";
/// Model free có thể chậm (K17: từng thấy chờ trên 60s) — trần hào phóng cho từng event.
const EVENT_TIMEOUT: Duration = Duration::from_secs(120);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn next_event(socket: &mut Socket) -> ServerMsg {
    loop {
        let message = tokio::time::timeout(EVENT_TIMEOUT, socket.next())
            .await
            .expect("hết thời gian chờ event WebSocket")
            .expect("kết nối WebSocket đứt")
            .expect("lỗi WebSocket");
        if let WsMessage::Text(text) = message {
            return serde_json::from_str(text.as_str()).expect("event không phải JSON");
        }
    }
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(WsMessage::Text(message.to_string().into()))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "gọi NVIDIA thật — cần mạng + NVIDIA_KEY"]
async fn m17_streaming_reaches_websocket_as_multiple_deltas() {
    let key = std::env::var(KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("chưa đặt {KEY_ENV} — nạp secrets.env rồi chạy lại"));
    // Key chỉ được truyền qua env của tiến trình con; không in ra bất cứ đâu.
    let _ = key;

    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let origin = format!("http://{addr}");

    let mut config = Config::default();
    config.data.dir = data.clone();
    config.agent.workspace = workspace;
    config.agent.allowed_users = vec!["web:admin".into()];
    config.tools.enabled = vec![]; // lượt thuần text — không tool, không confirm
    config.learning.enabled = false;
    config.telegram.enabled = false;
    config.web.public_origin = origin.clone();
    config.web.bind = addr;
    config.llm.provider = LlmProviderKind::OpenAiCompat;
    config.llm.model = MODEL.to_string();
    config.llm.base_url = Some(BASE_URL.to_string());
    config.llm.api_key_env = KEY_ENV.to_string();
    set_password(&data, PASSWORD, None).unwrap();
    let config_path = temp.path().join("bean.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bean"))
        .arg("--config")
        .arg(&config_path)
        .arg("serve")
        .current_dir(temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut ready = false;
    for _ in 0..200 {
        if TcpStream::connect(addr).await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(ready, "bean serve không khởi động");

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{origin}/api/auth/login"))
        .header(ORIGIN, &origin)
        .header(CONTENT_TYPE, "application/json")
        .body(json!({ "password": PASSWORD }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let response = client
        .post(format!("{origin}/api/sessions"))
        .header(ORIGIN, &origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({ "title": "M17 streaming thật" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let session: Value = response.json().await.unwrap();
    let session_id = session["id"].as_i64().unwrap();

    let request = Request::builder()
        .uri(format!("ws://{addr}/api/ws"))
        .header("Origin", &origin)
        .header("Host", addr.to_string())
        .header("Cookie", &cookie)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .body(())
        .unwrap();
    let mut socket = connect_async(request).await.unwrap().0;
    assert!(matches!(
        next_event(&mut socket).await,
        ServerMsg::Sync { .. }
    ));

    send(
        &mut socket,
        json!({
            "type": "start",
            "session_id": session_id,
            "text": "Đếm từ 1 đến 5, mỗi số nằm trên một dòng riêng, không viết thêm gì khác."
        }),
    )
    .await;

    // Đọc event đến khi Final: đếm delta text **không rỗng, không reset**.
    // Final kết thúc vòng lặp nên `delta_count` lúc break chính là số delta trước Final.
    let mut delta_count = 0usize;
    let mut streamed_text = String::new();
    loop {
        match next_event(&mut socket).await {
            ServerMsg::TextDelta { text, reset, .. } => {
                if !reset && !text.is_empty() {
                    delta_count += 1;
                    streamed_text.push_str(&text);
                }
            }
            ServerMsg::Final { .. } => break,
            ServerMsg::Error { code, message, .. } => {
                panic!("run lỗi trên dây thật: {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        delta_count >= 3,
        "chỉ nhận {delta_count} delta text trước Final — stream không tiến triển dần qua WS"
    );
    assert!(
        streamed_text.contains('1') && streamed_text.contains('5'),
        "text ghép từ delta sai: {streamed_text}"
    );
    println!(
        "m17 streaming thật: {delta_count} delta text qua WS, {} chars",
        streamed_text.chars().count()
    );

    // Dừng êm như mọi E2E khác — SIGTERM phải thoát sạch.
    let pid = child.id().unwrap().to_string();
    assert!(
        tokio::process::Command::new("kill")
            .args(["-TERM", pid.as_str()])
            .status()
            .await
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "SIGTERM phải shutdown êm: {status:?}");
}
