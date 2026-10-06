//! Vong lap WebSocket `/api/ws` — noi chan duy nhat giua UI va Router (muc 11.2).
//!
//! # Bat bien quan trong nhat
//!
//! * Run **thuoc Router**, khong thuoc ket noi: dong tab hay rot WebSocket KHONG huuy
//!   run; confirm dang cho van doi het `timeout_seconds` roi moi thanh DENY (muc 10).
//! * Kiem tra `Origin` + cookie **truoc** khi nang cap len (chong cross-site WebSocket
//!   hijacking, muc 22.15) — phan do nam o `guard.rs`, phan verify token nam o day.
//! * Khi WS noi lai, gui `Sync` de UI khoi phuc run dang chay va confirm dang cho —
//!   khong co ban ghi su kien. Xem `sync_msg`.
//!
//! # Vi sao tach rieng
//!
//! Day la **may trang thai** (select! giua socket, broadcast, timer), hoan toan khac
//! ban chat request/response cua REST. Tron hai loai do se lam cam thay co so ket noi
//! WS vao mot handler REST.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;
use bean_core::{Decision, Incoming, Router};
use bean_types::{Config, Risk, RunEvent, SessionId};
use tokio::sync::{OwnedSemaphorePermit, broadcast};

use crate::api_types::{ClientMsg, DecisionDto, PendingConfirm, RiskDto, RunningInfo, ServerMsg};

use super::{
    ApiFailure, MAX_WS_MESSAGE_BYTES, WS_HEARTBEAT, WS_IDLE_TIMEOUT, WebState, cookie_token,
};
/// WebSocket handler: xác thực trước khi `on_upgrade`, rồi mỗi socket có receiver riêng.
pub(super) async fn ws_handler(
    State(state): State<WebState>,
    jar: CookieJar,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(token) = cookie_token(&jar) else {
        return ApiFailure::unauthorized().into_response();
    };
    let user_id = match state.auth.authenticate_token(&token).await {
        // So khớp với `[web].user_id` đang cấu hình: session phải thuộc đúng user mà
        // instance này phục vụ, nếu không một phiên cũ của user khác có thể lọt vào.
        Ok(info) if info.user_id == state.config.web.user_id => info.user_id,
        _ => return ApiFailure::unauthorized().into_response(),
    };
    let permit = match Arc::clone(&state.ws_connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return ApiFailure::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too_many_connections",
                "đã đạt giới hạn kết nối WebSocket",
            )
            .into_response();
        }
    };
    upgrade
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_socket(socket, state, user_id, permit))
}

async fn run_socket(
    mut socket: WebSocket,
    state: WebState,
    user_id: String,
    _connection_permit: OwnedSemaphorePermit,
) {
    let mut router_events = state.router.events();
    let mut notifications = state.notifications.subscribe();
    let mut heartbeat = tokio::time::interval(WS_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_activity = Instant::now();
    if send_msg(
        &mut socket,
        &sync_msg(&state.router, &state.config, &user_id),
    )
    .await
    .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            event = router_events.recv() => match event {
                Ok(event) => {
                    if let Some(message) = map_run_event(event) {
                        // Lọc theo vai trò của người xem trước khi gửi: `prompt` của
                        // confirm có thể chứa nguyên văn lệnh shell, nên đây là dữ
                        // liệu nhạy cảm, không phải thứ để gửi rồi cho client ẩn.
                        if let Some(role) = event_role(&message)
                            && !event_visible_to(&state.config, &user_id, role)
                        {
                            last_activity = Instant::now();
                            continue;
                        }
                        if send_msg(&mut socket, &message).await.is_err() { break; }
                    }
                    last_activity = Instant::now();
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "WebSocket bị lag; gửi Sync mới");
                    if send_msg(&mut socket, &sync_msg(&state.router, &state.config, &user_id)).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            event = notifications.recv() => {
                match event {
                    Ok(message) => {
                        if send_msg(&mut socket, &message).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "WebSocket notification bị lag; gửi Sync mới");
                        if send_msg(&mut socket, &sync_msg(&state.router, &state.config, &user_id)).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            },
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                last_activity = Instant::now();
                match message {
                    WsMessage::Text(text) => {
                        if text.as_str().len() > MAX_WS_MESSAGE_BYTES {
                            let _ = send_msg(
                                &mut socket,
                                &ServerMsg::Error {
                                    session_id: None,
                                    run_id: None,
                                    code: "message_too_large".into(),
                                    message: "message vượt giới hạn".into(),
                                },
                            )
                            .await;
                            break;
                        }
                        match serde_json::from_str::<ClientMsg>(text.as_str()) {
                            Ok(ClientMsg::Ping) => {
                                if send_msg(&mut socket, &ServerMsg::Pong).await.is_err() { break; }
                            }
                            Ok(ClientMsg::Start { session_id, text }) => {
                                if text.len() > MAX_WS_MESSAGE_BYTES {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: Some(session_id),
                                        run_id: None,
                                        code: "text_too_large".into(),
                                        message: "nội dung vượt giới hạn".into(),
                                    }).await;
                                } else if handle_start(&state, &user_id, session_id, text).await.is_err() {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: Some(session_id),
                                        run_id: None,
                                        code: "invalid_session".into(),
                                        message: "session không hợp lệ".into(),
                                    }).await;
                                }
                            }
                            Ok(ClientMsg::Cancel { session_id }) => {
                                if handle_cancel(&state, &user_id, session_id).await.is_err() {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: Some(session_id),
                                        run_id: None,
                                        code: "invalid_session".into(),
                                        message: "session không hợp lệ".into(),
                                    }).await;
                                }
                            }
                            Ok(ClientMsg::Confirm { confirm_id, decision }) => {
                                let decision = match decision {
                                    DecisionDto::Allow => Decision::Allow,
                                    DecisionDto::AllowInSession => Decision::AllowInSession,
                                    DecisionDto::Deny => Decision::Deny,
                                };
                                if state.router.resolve_confirm(&confirm_id, decision, &user_id).await.is_err() {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: None,
                                        run_id: None,
                                        code: "confirm_invalid".into(),
                                        message: "confirm không hợp lệ".into(),
                                    }).await;
                                }
                            }
                            Err(_) => {
                                let _ = send_msg(&mut socket, &ServerMsg::Error {
                                    session_id: None,
                                    run_id: None,
                                    code: "invalid_message".into(),
                                    message: "message không hợp lệ".into(),
                                }).await;
                            }
                        }
                    }
                    WsMessage::Ping(payload) => {
                        if socket.send(WsMessage::Pong(payload)).await.is_err() { break; }
                    }
                    WsMessage::Close(_) => break,
                    WsMessage::Binary(_) => {
                        let _ = send_msg(&mut socket, &ServerMsg::Error {
                            session_id: None,
                            run_id: None,
                            code: "binary_not_allowed".into(),
                            message: "WebSocket chỉ nhận JSON text".into(),
                        }).await;
                    }
                    WsMessage::Pong(_) => {}
                }
            }
            _ = heartbeat.tick() => {
                if socket.send(WsMessage::Ping(Default::default())).await.is_err() { break; }
                if last_activity.elapsed() > WS_IDLE_TIMEOUT { break; }
            }
        }
    }
}

async fn handle_start(
    state: &WebState,
    user_id: &str,
    session_id: i64,
    text: String,
) -> Result<(), ()> {
    let info = state
        .store
        .session_info(SessionId::new(session_id))
        .await
        .map_err(|_| ())?
        .ok_or(())?;
    if info.user_id != user_id || info.channel != "web" || info.archived {
        return Err(());
    }
    state
        .router
        .submit(
            Incoming::new("web", user_id, user_id, text).with_session(SessionId::new(session_id)),
        )
        .await
        .map_err(|_| ())?;
    Ok(())
}

async fn handle_cancel(state: &WebState, user_id: &str, session_id: i64) -> Result<(), ()> {
    let info = state
        .store
        .session_info(SessionId::new(session_id))
        .await
        .map_err(|_| ())?
        .ok_or(())?;
    if info.user_id != user_id || info.channel != "web" {
        return Err(());
    }
    state
        .router
        .cancel_session(SessionId::new(session_id))
        .await;
    Ok(())
}

/// Dựng `Sync` cho **một** người xem, đã lọc theo RBAC của họ.
///
/// # Vì sao phải lọc
///
/// `Sync` vốn broadcast cho mọi kết nối. Khi web chỉ có một user `web:admin` thì
/// vô hại, nhưng nếu không lọc thì đây là **đường rò dữ liệu chéo vai trò**:
/// `finance-readonly` sẽ nhận `run_id` và cả `prompt` (nguyên văn lệnh shell) của
/// run thuộc Developer/Security-scan — đúng loại rò mà `docs/security-review.md`
/// đã cảnh báo cho `/api/audit`.
///
/// Lọc tại đây (trước khi tuần tự hoá) giữ đúng nguyên tắc: dữ liệu không thuộc
/// quyền thì **không có trong payload**, không phải "UI không hiển thị".
///
/// `None` nghĩa là RBAC tắt — không có khái niệm agent con nào để phân quyền, nên
/// ai cũng thấy (giữ nguyên hành vi một-người-dùng của M21).
fn sync_msg(router: &Router, config: &Config, user_id: &str) -> ServerMsg {
    let permissions = config.permissions_for(user_id);
    let visible = |role: &Option<String>| match role {
        None => true,
        Some(name) => config
            .role(name)
            .is_none_or(|role| permissions.can_view_agent(&role.tag_set())),
    };
    let snapshot = router.snapshot();
    ServerMsg::Sync {
        running: snapshot
            .running
            .into_iter()
            .filter(|item| visible(&item.role))
            .map(|item| RunningInfo {
                session_id: item.session_id.get(),
                run_id: item.run_id.as_str().to_string(),
                role: item.role,
            })
            .collect(),
        pending_confirms: snapshot
            .pending_confirms
            .into_iter()
            .filter(|item| visible(&item.role))
            .map(|item| PendingConfirm {
                confirm_id: item.confirm_id.as_str().to_string(),
                session_id: item.session_id.get(),
                run_id: item.run_id.as_str().to_string(),
                prompt: item.prompt,
                risk: match item.risk {
                    Risk::Safe => RiskDto::Safe,
                    Risk::Confirm => RiskDto::Confirm,
                    Risk::Dangerous => RiskDto::Dangerous,
                },
                allow_session_option: item.allow_session_option,
                timeout_seconds: item.timeout_seconds,
                role: item.role,
            })
            .collect(),
    }
}

/// Sự kiệp run này có thuộc vai trò mà `user_id` được phép thấy không?
///
/// `map_run_event` không có ngữ cảnh người xem nên không thể lọc; hàm này chạy ở
/// tầng socket, nơi đã xác thực `user_id`. Dùng cho sự kiện **trực tiếp** (không
/// qua `Sync`) để đường rò dữ liệu chéo vai trò không mở lại qua kênh broadcast.
fn event_visible_to(config: &Config, user_id: &str, role: &Option<String>) -> bool {
    let Some(name) = role else {
        return true;
    };
    let permissions = config.permissions_for(user_id);
    config
        .role(name)
        .is_none_or(|role| permissions.can_view_agent(&role.tag_set()))
}

/// Sự kiện `ConfirmRequest` mang `role`; các sự kiện khác không (chúng không chứa
/// dữ liệu thuộc domain của agent con) nên đi qua không cần lọc.
fn event_role(message: &ServerMsg) -> Option<&Option<String>> {
    match message {
        ServerMsg::ConfirmRequest { role, .. } => Some(role),
        _ => None,
    }
}

fn map_run_event(event: RunEvent) -> Option<ServerMsg> {
    Some(match event {
        RunEvent::Queued {
            session_id,
            run_id,
            position,
        } => ServerMsg::Queued {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            position,
        },
        RunEvent::Text {
            session_id,
            run_id,
            text,
        } => ServerMsg::Text {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            text,
        },
        RunEvent::TextDelta {
            session_id,
            run_id,
            text,
            index,
            reset,
        } => ServerMsg::TextDelta {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            text,
            index,
            reset,
        },
        RunEvent::ToolStart {
            session_id,
            run_id,
            id,
            tool,
            summary,
            args_preview,
            ..
        } => ServerMsg::ToolStart {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            id,
            tool,
            summary,
            args_preview,
        },
        RunEvent::ToolEnd {
            session_id,
            run_id,
            id,
            ok,
            output_preview,
            ..
        } => ServerMsg::ToolEnd {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            id,
            ok,
            output_preview,
        },
        RunEvent::ConfirmRequest {
            session_id,
            run_id,
            confirm_id,
            prompt,
            risk,
            allow_session_option,
            timeout_seconds,
            role,
        } => ServerMsg::ConfirmRequest {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            confirm_id: confirm_id.as_str().into(),
            prompt,
            risk: match risk {
                Risk::Safe => RiskDto::Safe,
                Risk::Confirm => RiskDto::Confirm,
                Risk::Dangerous => RiskDto::Dangerous,
            },
            allow_session_option,
            timeout_seconds,
            role,
        },
        RunEvent::ConfirmResolved {
            confirm_id,
            outcome,
            ..
        } => ServerMsg::ConfirmResolved {
            confirm_id: confirm_id.as_str().into(),
            outcome: match outcome {
                bean_types::ConfirmOutcome::Allowed => "allowed".into(),
                bean_types::ConfirmOutcome::Denied => "denied".into(),
                bean_types::ConfirmOutcome::Expired => "expired".into(),
            },
        },
        RunEvent::Final {
            session_id,
            run_id,
            message_id,
            ..
        } => ServerMsg::Final {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            message_id: message_id.unwrap_or_default(),
        },
        RunEvent::Error {
            session_id,
            run_id,
            code,
            message,
        } => ServerMsg::Error {
            session_id: Some(session_id.get()),
            run_id: Some(run_id.as_str().into()),
            code,
            message,
        },
    })
}

async fn send_msg(socket: &mut WebSocket, message: &ServerMsg) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    socket.send(WsMessage::text(text)).await.map_err(|_| ())
}
