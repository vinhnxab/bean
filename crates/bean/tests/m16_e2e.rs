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
    current_dir: &std::path::Path,
) -> (Child, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bean"))
        .arg("--config")
        .arg(config_path)
        .arg("serve")
        .arg("--fake-llm")
        .arg(script_path)
        .current_dir(current_dir)
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

/// SIGTERM rồi chờ tiến trình thoát với mã 0 — shutdown êm phải là hành vi mặc định.
async fn stop_server(child: &mut Child) {
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

/// Môi trường chung cho E2E K11/K18: dữ liệu + workspace trong temp, không Telegram,
/// địa chỉ loopback ngẫu nhiên. `current_dir` của binary cũng nằm trong temp — serve
/// đặt thư mục draft skills ở `skills/_drafts` tính theo CWD nên không thể để kế thừa
/// CWD của test (sẽ ghi bẩn repo).
struct E2eEnv {
    temp: tempfile::TempDir,
    config_path: std::path::PathBuf,
    script_path: std::path::PathBuf,
    addr: SocketAddr,
    origin: String,
}

async fn setup_env(tools: &[&str], learning_enabled: bool) -> E2eEnv {
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
    config.tools.enabled = tools.iter().map(|name| (*name).to_string()).collect();
    config.learning.enabled = learning_enabled;
    config.learning.min_tool_calls = 5;
    config.learning.proposal_interval_minutes = 60;
    config.telegram.enabled = false;
    config.web.public_origin = origin.clone();
    config.web.bind = addr;
    set_password(&data, PASSWORD, None).unwrap();
    let config_path = temp.path().join("bean.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let script_path = temp.path().join("fake.json");
    E2eEnv {
        temp,
        config_path,
        script_path,
        addr,
        origin,
    }
}

fn write_script(path: &std::path::Path, script: Value) {
    std::fs::write(path, script.to_string()).unwrap();
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
    set_password(&data, PASSWORD, None).unwrap();
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
    let (mut child, addr) = start_server(&config_path, &script_path, addr, temp.path()).await;
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
    stop_server(&mut child).await;
}

/// (D25) Kế hoạch bước phải tới được **trên dây WebSocket** từ binary thật với
/// kịch bản fake-llm gọi `update_plan`. Regress: whitelist biên TS thiếu case
/// `plan` nuốt im lặng event — server phát đúng nhưng UI không bao giờ thấy;
/// test này chốt phía server của sợi dây (phía client chốt ở ws.test.ts).
#[tokio::test]
async fn plan_events_reach_the_ws_wire_from_real_binary() {
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
    set_password(&data, PASSWORD, None).unwrap();
    let config_path = temp.path().join("bean.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let script_path = temp.path().join("fake.json");
    let script = json!({ "responses": [
        {
            "tool_calls": [{ "id": "p1", "name": "update_plan", "args": { "steps": [
                { "title": "Xem workspace", "status": "done" },
                { "title": "Tạo ghi chú", "status": "in_progress" },
                { "title": "Đọc lại", "status": "pending" }
            ] } }],
            "stop": "tool_use"
        },
        {
            "tool_calls": [{ "id": "p2", "name": "list_dir", "args": {} }],
            "stop": "tool_use"
        },
        {
            "tool_calls": [{ "id": "p3", "name": "update_plan", "args": { "steps": [
                { "title": "Xem workspace", "status": "done" },
                { "title": "Tạo ghi chú", "status": "done" },
                { "title": "Đọc lại", "status": "done" }
            ] } }],
            "stop": "tool_use"
        },
        { "text": "hoàn thành cả ba bước", "stop": "end_turn" }
    ] });
    std::fs::write(&script_path, script.to_string()).unwrap();
    let (child, addr) = start_server(&config_path, &script_path, addr, temp.path()).await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &origin).await;
    let response = client
        .post(format!("{origin}/api/sessions"))
        .header(ORIGIN, &origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({ "title": "Plan E2E" }).to_string())
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
        json!({ "type": "start", "session_id": session_id, "text": "làm kế hoạch" }),
    )
    .await;

    // Gom mọi frame plan cho tới Final: kỳ vọng 2 snapshot đúng nội dung.
    let mut plans: Vec<Vec<(String, String)>> = Vec::new();
    loop {
        match next_event(&mut socket).await {
            ServerMsg::Plan { steps, .. } => {
                plans.push(
                    steps
                        .into_iter()
                        .map(|step| {
                            let status = match step.status {
                                bean_web::PlanStepStatusDto::Pending => "pending",
                                bean_web::PlanStepStatusDto::InProgress => "in_progress",
                                bean_web::PlanStepStatusDto::Done => "done",
                            };
                            (step.title, status.to_string())
                        })
                        .collect(),
                );
            }
            ServerMsg::Final { .. } => break,
            _ => {}
        }
    }
    assert_eq!(
        plans.len(),
        2,
        "phải nhận đúng 2 frame plan trên dây, nhận được: {plans:?}"
    );
    assert_eq!(
        plans[0],
        vec![
            ("Xem workspace".into(), "done".into()),
            ("Tạo ghi chú".into(), "in_progress".into()),
            ("Đọc lại".into(), "pending".into())
        ]
    );
    assert_eq!(
        plans[1],
        vec![
            ("Xem workspace".into(), "done".into()),
            ("Tạo ghi chú".into(), "done".into()),
            ("Đọc lại".into(), "done".into())
        ]
    );

    let pid = child.id().unwrap().to_string();
    let _ = tokio::process::Command::new("kill")
        .args(["-TERM", pid.as_str()])
        .status()
        .await
        .unwrap()
        .success();
    drop(socket);
}

/// (D26) Checkpoint/rollback phải chạy được với binary thật: model ghi file (duyệt
/// confirm) rồi gọi `rollback_workspace` (duyệt confirm lần 2), workspace trở về
/// trạng thái ngay trước khi ghi — file sinh sau checkpoint bị xoá, không còn dấu vết.
#[tokio::test]
async fn rollback_restores_workspace_in_real_binary() {
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
    config.agent.workspace = workspace.clone();
    config.agent.allowed_users = vec!["web:admin".into()];
    config.tools.enabled = vec!["files".into()];
    config.learning.enabled = false;
    config.telegram.enabled = false;
    config.web.public_origin = origin.clone();
    config.web.bind = addr;
    set_password(&data, PASSWORD, None).unwrap();
    let config_path = temp.path().join("bean.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let script_path = temp.path().join("fake.json");
    let script = json!({ "responses": [
        { "tool_calls": [{ "id": "c1", "name": "write_file", "args": { "path": "note.txt", "content": "bản ghi nhầm" } }], "stop": "tool_use" },
        { "tool_calls": [{ "id": "c2", "name": "rollback_workspace", "args": {} }], "stop": "tool_use" },
        { "text": "đã hoàn tác", "stop": "end_turn" }
    ] });
    std::fs::write(&script_path, script.to_string()).unwrap();
    let (child, addr) = start_server(&config_path, &script_path, addr, temp.path()).await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &origin).await;
    let response = client
        .post(format!("{origin}/api/sessions"))
        .header(ORIGIN, &origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({ "title": "Rollback E2E" }).to_string())
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
        json!({ "type": "start", "session_id": session_id, "text": "ghi rồi hoàn tác" }),
    )
    .await;

    // Hai confirm theo thứ tự: write_file rồi rollback_workspace — duyệt cả hai.
    for _ in 0..2 {
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
    }
    wait_for(&mut socket, |event| {
        matches!(event, ServerMsg::Final { .. })
    })
    .await;

    // Workspace trở về trạng thái trước lượt: file sinh sau checkpoint đã bị xoá.
    assert!(
        !workspace.join("note.txt").exists(),
        "file sinh sau checkpoint phải bị xoá khi rollback"
    );
    // Checkpoint nằm ở data.dir/checkpoints — ngoài jail workspace.
    assert!(
        data.join("checkpoints").read_dir().unwrap().count() > 0,
        "phải có ít nhất một snapshot được chụp"
    );
    // Tool result của rollback qua REST phải báo khôi phục thành công.
    let messages: Value = client
        .get(format!("{origin}/api/sessions/{session_id}/messages"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = messages.to_string();
    assert!(
        text.contains("Đã khôi phục workspace"),
        "tool result rollback phải báo khôi phục: {text}"
    );
    drop(socket);
    let pid = child.id().unwrap().to_string();
    let _ = tokio::process::Command::new("kill")
        .args(["-TERM", pid.as_str()])
        .status()
        .await
        .unwrap()
        .success();
}

/// (K11) Bộ nhớ bền vững xuyên qua binary thật: `memory_save` ở tiến trình 1,
/// SIGTERM + khởi động lại với cùng `data.dir`, rồi `memory_search` ở tiến trình 2
/// phải tìm thấy ghi nhớ — thay cho fixture script tạm trong `/tmp` trước đây.
#[tokio::test]
async fn memory_survives_restart_in_real_binary() {
    let env = setup_env(&["files", "memory"], false).await;
    write_script(
        &env.script_path,
        json!({ "responses": [
            { "tool_calls": [{ "id": "m1", "name": "memory_save", "args": { "text": "Người dùng thích cà phê sữa đá vào buổi sáng.", "tags": "e2e,ưu tiên" } }], "stop": "tool_use" },
            { "text": "đã ghi nhớ", "stop": "end_turn" }
        ] }),
    );
    let (mut child, addr) = start_server(
        &env.config_path,
        &env.script_path,
        env.addr,
        env.temp.path(),
    )
    .await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &env.origin).await;
    let response = client
        .post(format!("{}/api/sessions", env.origin))
        .header(ORIGIN, &env.origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({ "title": "K11 Memory E2E" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let session: Value = response.json().await.unwrap();
    let session_id = session["id"].as_i64().unwrap();
    let mut socket = connect_ws(addr, &env.origin, &cookie).await;
    assert!(matches!(
        next_event(&mut socket).await,
        ServerMsg::Sync { .. }
    ));

    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "ghi nhớ sở thích cà phê" }),
    )
    .await;
    let start = wait_for(
        &mut socket,
        |event| matches!(event, ServerMsg::ToolStart { tool, .. } if tool == "memory_save"),
    )
    .await;
    let ServerMsg::ToolStart { id: save_call, .. } = start else {
        unreachable!()
    };
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::ToolEnd { id, ok: true, .. } if *id == save_call
        ))
        .await,
        ServerMsg::ToolEnd { .. }
    ));
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Final { .. }
        ))
        .await,
        ServerMsg::Final { .. }
    ));
    stop_server(&mut child).await;

    // Tiến trình 2: cùng data.dir — ghi nhớ và lịch sử phiên phải còn nguyên.
    write_script(
        &env.script_path,
        json!({ "responses": [
            { "tool_calls": [{ "id": "m2", "name": "memory_search", "args": { "query": "cà phê" } }], "stop": "tool_use" },
            { "text": "đã tìm", "stop": "end_turn" }
        ] }),
    );
    let (mut child, addr) =
        start_server(&env.config_path, &env.script_path, addr, env.temp.path()).await;
    let cookie = login(&client, &env.origin).await;
    let sessions: Value = client
        .get(format!("{}/api/sessions", env.origin))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        sessions["sessions"].as_array().unwrap().len(),
        1,
        "phiên cũ phải còn sau restart"
    );
    assert_eq!(sessions["sessions"][0]["id"].as_i64().unwrap(), session_id);
    assert_eq!(sessions["sessions"][0]["title"], "K11 Memory E2E");

    let mut socket = connect_ws(addr, &env.origin, &cookie).await;
    assert!(matches!(
        next_event(&mut socket).await,
        ServerMsg::Sync { .. }
    ));
    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "tìm ghi nhớ cà phê" }),
    )
    .await;
    let start = wait_for(
        &mut socket,
        |event| matches!(event, ServerMsg::ToolStart { tool, .. } if tool == "memory_search"),
    )
    .await;
    let ServerMsg::ToolStart {
        id: search_call, ..
    } = start
    else {
        unreachable!()
    };
    let found = wait_for(&mut socket, |event| {
        matches!(
            event,
            ServerMsg::ToolEnd { id, ok: true, .. } if *id == search_call
        )
    })
    .await;
    let ServerMsg::ToolEnd { output_preview, .. } = found else {
        unreachable!()
    };
    assert!(
        output_preview.contains("cà phê sữa đá"),
        "memory_search ở tiến trình mới phải tìm thấy ghi nhớ đã lưu: {output_preview}"
    );
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Final { .. }
        ))
        .await,
        ServerMsg::Final { .. }
    ));

    // REST đọc trực tiếp SQLite của tiến trình mới: ghi nhớ vẫn nằm trong DB.
    let memories: Value = client
        .get(format!("{}/api/memories?q=cà phê", env.origin))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(memories["memories"].as_array().unwrap().len(), 1);
    assert_eq!(memories["memories"][0]["tags"], "e2e,ưu tiên");
    drop(socket);
    stop_server(&mut child).await;
}

/// (K18) Learning loop xuyên qua binary/server thật: run ≥ 5 tool call → notification →
/// xem draft/diff qua REST → loader CHƯA thấy skill → duyệt → skill xuất hiện → run kế
/// tiếp nạp được skill. Trước đây chỉ có unit test trong tiến trình.
#[tokio::test]
async fn learning_draft_approval_roundtrip_in_real_binary() {
    let env = setup_env(&["files", "skills"], true).await;
    let proposal_text = json!({
        "reusable": true,
        "proposal": {
            "kind": "new",
            "name": "e2e-reusable-skill",
            "description": "Dùng khi cần lặp lại quy trình E2E.",
            "body": "# Reusable guide\n\n1. Kiểm tra đầu vào.\n2. Làm từng bước.",
            "reason": "Run thành công sau nhiều bước có thể lặp lại."
        }
    })
    .to_string();
    write_script(
        &env.script_path,
        json!({ "responses": [
            { "tool_calls": [
                { "id": "t1", "name": "list_dir", "args": {} },
                { "id": "t2", "name": "list_dir", "args": {} },
                { "id": "t3", "name": "list_dir", "args": {} },
                { "id": "t4", "name": "list_dir", "args": {} },
                { "id": "t5", "name": "list_dir", "args": {} }
            ], "stop": "tool_use" },
            { "text": "hoàn tất", "stop": "end_turn" },
            { "text": proposal_text, "stop": "end_turn" },
            { "tool_calls": [{ "id": "l1", "name": "load_skill", "args": { "name": "e2e-reusable-skill" } }], "stop": "tool_use" },
            { "text": "đã nạp skill", "stop": "end_turn" }
        ] }),
    );
    let (mut child, addr) = start_server(
        &env.config_path,
        &env.script_path,
        env.addr,
        env.temp.path(),
    )
    .await;
    let client = reqwest::Client::new();
    let cookie = login(&client, &env.origin).await;
    let response = client
        .post(format!("{}/api/sessions", env.origin))
        .header(ORIGIN, &env.origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({ "title": "K18 Learning E2E" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let session: Value = response.json().await.unwrap();
    let session_id = session["id"].as_i64().unwrap();
    let mut socket = connect_ws(addr, &env.origin, &cookie).await;
    assert!(matches!(
        next_event(&mut socket).await,
        ServerMsg::Sync { .. }
    ));

    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "làm quy trình nhiều bước" }),
    )
    .await;
    let mut tool_ends = 0;
    loop {
        match wait_for(&mut socket, |event| {
            matches!(
                event,
                ServerMsg::ToolEnd { .. }
                    | ServerMsg::Final { .. }
                    | ServerMsg::Notification { .. }
            )
        })
        .await
        {
            ServerMsg::ToolEnd { ok: true, .. } => tool_ends += 1,
            ServerMsg::Final { .. } => break,
            _ => panic!("không mong đợi Notification trước Final"),
        }
    }
    assert_eq!(
        tool_ends, 5,
        "run phải chạy đủ 5 tool call trước reflection"
    );
    // Notification báo draft chờ duyệt — gửi sau Final khi reflection xong.
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Notification { .. }
        ))
        .await,
        ServerMsg::Notification { .. }
    ));

    // Draft xem được qua REST kèm diff/nội dung.
    let drafts: Value = client
        .get(format!("{}/api/skills/drafts", env.origin))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(drafts["drafts"].as_array().unwrap().len(), 1);
    assert_eq!(drafts["drafts"][0]["name"], "e2e-reusable-skill");
    assert_eq!(drafts["drafts"][0]["status"], "pending");
    assert!(
        drafts["drafts"][0]["content"]
            .as_str()
            .unwrap()
            .contains("# Reusable guide"),
        "draft phải kèm nội dung để xem trước khi duyệt"
    );
    let draft_id = drafts["drafts"][0]["id"].as_str().unwrap().to_string();

    // Loader chưa thấy skill trước khi duyệt.
    let skills: Value = client
        .get(format!("{}/api/skills", env.origin))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !skills.to_string().contains("e2e-reusable-skill"),
        "skill chưa duyệt không được xuất hiện trong catalog: {skills}"
    );

    // Duyệt qua REST — quyết định phải nằm trong audit JSONL với actor (K19).
    let approved: Value = client
        .post(format!(
            "{}/api/skills/drafts/{draft_id}/approve",
            env.origin
        ))
        .header(ORIGIN, &env.origin)
        .header(CONTENT_TYPE, "application/json")
        .header("Cookie", &cookie)
        .body(json!({}).to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(approved["status"], "approved");
    let audit_text =
        std::fs::read_to_string(env.temp.path().join("data/audit/audit.jsonl")).unwrap();
    assert!(
        audit_text.contains("skill_draft_approve") && audit_text.contains("web:admin"),
        "audit phải ghi quyết định duyệt với actor: {audit_text}"
    );

    // Skill đã kích hoạt: có trong danh sách và đọc được nội dung đầy đủ.
    let skills: Value = client
        .get(format!("{}/api/skills", env.origin))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(skills.to_string().contains("e2e-reusable-skill"));
    let detail: Value = client
        .get(format!("{}/api/skills/e2e-reusable-skill", env.origin))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        detail["content"]
            .as_str()
            .unwrap()
            .contains("# Reusable guide")
    );

    // Run kế tiếp nạp được skill vừa duyệt qua binary thật.
    send(
        &mut socket,
        json!({ "type": "start", "session_id": session_id, "text": "nạp skill vừa tạo" }),
    )
    .await;
    let start = wait_for(
        &mut socket,
        |event| matches!(event, ServerMsg::ToolStart { tool, .. } if tool == "load_skill"),
    )
    .await;
    let ServerMsg::ToolStart { id: load_call, .. } = start else {
        unreachable!()
    };
    let loaded = wait_for(&mut socket, |event| {
        matches!(
            event,
            ServerMsg::ToolEnd { id, ok: true, .. } if *id == load_call
        )
    })
    .await;
    let ServerMsg::ToolEnd { output_preview, .. } = loaded else {
        unreachable!()
    };
    assert!(
        output_preview.contains("Reusable guide"),
        "load_skill phải đọc được skill đã duyệt: {output_preview}"
    );
    assert!(matches!(
        wait_for(&mut socket, |event| matches!(
            event,
            ServerMsg::Final { .. }
        ))
        .await,
        ServerMsg::Final { .. }
    ));
    drop(socket);
    stop_server(&mut child).await;
}
