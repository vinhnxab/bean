//! M16 smoke E2E: binary `serve --fake-llm`, auth, confirm, Stop và reconnect WebSocket.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;

use bean_types::Config;
use bean_web::{ServerMsg, set_password};
use futures_util::{SinkExt, StreamExt};
use reqwest::header::{CONTENT_TYPE, ORIGIN, SET_COOKIE};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message as WsMessage, http::Request},
};

const PASSWORD: &str = "e2e-password-123456";
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn next_event(socket: &mut Socket) -> ServerMsg {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let WsMessage::Text(text) = message {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(WsMessage::Text(message.to_string().into()))
        .await
        .unwrap();
}

async fn wait_for(socket: &mut Socket, predicate: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
    for _ in 0..40 {
        let event = next_event(socket).await;
        if predicate(&event) {
            return event;
        }
    }
    panic!("hết thời gian chờ event WebSocket");
}

async fn connect_ws(addr: SocketAddr, origin: &str, cookie: &str) -> Socket {
    let request = Request::builder()
        .uri(format!("ws://{addr}/api/ws"))
        .header("Origin", origin)
        .header("Host", addr.to_string())
        .header("Cookie", cookie)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .body(())
        .unwrap();
    connect_async(request).await.unwrap().0
}

async fn login(client: &reqwest::Client, origin: &str) -> String {
    let response = client
        .post(format!("{origin}/api/auth/login"))
        .header(ORIGIN, origin)
        .header(CONTENT_TYPE, "application/json")
        .body(json!({ "password": PASSWORD }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let set_cookie = response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    set_cookie.split(';').next().unwrap().to_string()
}

async fn start_server(
    config_path: &std::path::Path,
    script_path: &std::path::Path,
    addr: SocketAddr,
) -> (Child, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bean"))
        .arg("--config")
        .arg(config_path)
        .arg("serve")
        .arg("--fake-llm")
        .arg(script_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if TcpStream::connect(addr).await.is_ok() {
            return (child, addr);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let _ = child.kill().await;
    panic!("bean serve không khởi động");
}

#[tokio::test]
async fn m16_login_chat_confirm_stop_reconnect_and_final() {
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
    config.tools.enabled = vec!["files".into()];
    config.learning.enabled = false;
    config.telegram.enabled = false;
    config.web.public_origin = origin.clone();
    config.web.bind = addr;
    set_password(&data, PASSWORD).unwrap();
    let config_path = temp.path().join("bean.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let script_path = temp.path().join("fake.json");
    let script = json!({ "responses": [
        { "tool_calls": [{ "id": "c1", "name": "write_file", "args": { "path": "e2e.txt", "content": "ok" } }], "stop": "tool_use" },
        { "text": "xong", "stop": "end_turn" },
        { "tool_calls": [{ "id": "c2", "name": "write_file", "args": { "path": "e2e.txt", "content": "stop" } }], "stop": "tool_use" },
        { "tool_calls": [{ "id": "c3", "name": "write_file", "args": { "path": "e2e.txt", "content": "reconnect" } }], "stop": "tool_use" },
        { "text": "xong sau reconnect", "stop": "end_turn" }
    ] });
    std::fs::write(&script_path, script.to_string()).unwrap();
    let (mut child, addr) = start_server(&config_path, &script_path, addr).await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &origin).await;
    let response = client
        .post(format!("{origin}/api/sessions"))
        .header(ORIGIN, &origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({ "title": "M16 E2E" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let session: Value = response.json().await.unwrap();
    let session_id = session["id"].as_i64().unwrap();
    let mut socket = connect_ws(addr, &origin, &cookie).await;
    assert!(matches!(
        next_event(&mut socket).await,
        ServerMsg::Sync { .. }
    ));

    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "ghi file" }),
    )
    .await;
    let confirm = wait_for(&mut socket, |event| {
        matches!(event, ServerMsg::ConfirmRequest { .. })
    })
    .await;
    let ServerMsg::ConfirmRequest { confirm_id, .. } = confirm else {
        unreachable!()
    };
    send(
        &mut socket,
        json!({ "type": "confirm", "confirm_id": confirm_id, "decision": "allow" }),
    )
    .await;
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Final { .. }
        ))
        .await,
        ServerMsg::Final { .. }
    ));

    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "stop" }),
    )
    .await;
    wait_for(&mut socket, |event| {
        matches!(event, ServerMsg::ConfirmRequest { .. })
    })
    .await;
    send(
        &mut socket,
        json!({ "type": "cancel", "session_id": session_id }),
    )
    .await;
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Error { .. }
        ))
        .await,
        ServerMsg::Error { .. }
    ));

    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "reconnect" }),
    )
    .await;
    let pending = wait_for(&mut socket, |event| {
        matches!(event, ServerMsg::ConfirmRequest { .. })
    })
    .await;
    let ServerMsg::ConfirmRequest { confirm_id, .. } = pending else {
        unreachable!()
    };
    drop(socket);
    let mut socket = connect_ws(addr, &origin, &cookie).await;
    let ServerMsg::Sync {
        pending_confirms, ..
    } = next_event(&mut socket).await
    else {
        panic!("phải nhận Sync sau reconnect");
    };
    assert_eq!(pending_confirms.len(), 1);
    assert_eq!(pending_confirms[0].confirm_id, confirm_id);
    send(
        &mut socket,
        json!({ "type": "confirm", "confirm_id": confirm_id, "decision": "allow" }),
    )
    .await;
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Final { .. }
        ))
        .await,
        ServerMsg::Final { .. }
    ));
    let messages: Value = client
        .get(format!("{origin}/api/sessions/{session_id}/messages"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(messages.to_string().contains("xong sau reconnect"));
    drop(socket);
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
