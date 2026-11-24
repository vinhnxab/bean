//! Web Push (VAPID): gửi thông báo tới trình duyệt/Android **khi tab đã đóng**.
//!
//! # Kiến trúc (SRP — mỗi file một trách nhiệm)
//!
//! ```text
//! push/
//! ├─ mod.rs    dispatcher (nguồn sự kiện, điều kiện gửi) + HTTP sender
//! ├─ vapid.rs  định danh VAPID: khóa ES256, JWT, persist 0600
//! └─ ece.rs    mã hoá payload aes128gcm (RFC 8291)
//! ```
//!
//! # Quy tắc gửi (bắt buộc giữ khi sửa)
//!
//! * Chỉ push khi **không còn kết nối WebSocket nào** — tab đang mở thì in-page
//!   notification đã đủ, push trùng là nhiễu.
//! * Chỉ push sự kiện người dùng cần biết: `Final`, `ConfirmRequest`,
//!   `Notification` (scheduler/learning). Text/tool delta không push.
//! * Lọc RBAC như `ws.rs` (`event_visible_to`) — người nhận push là admin vì
//!   endpoint subscribe chỉ dành cho admin.
//! * Endpoint không hợp lệ (404/410) bị xoá khỏi store — push service nói rõ
//!   subscription đã chết, giữ lại chỉ làm mỗi lần gửi đều lỗi.
//! * URL endpoint là dữ liệu không tin cậy từ client: chỉ chấp nhận `https`,
//!   không log nguyên văn (chứa token định danh subscription).

mod ece;
mod vapid;

pub(crate) use ece::decode_b64url as decode_client_b64;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use bean_memory::PushSubscription;
use bean_types::{Config, RunEvent};
use sha2::Digest;
use tokio_util::sync::CancellationToken;
use url::Url;

pub use vapid::VapidKeys;
use vapid::vapid_keys_path;

use crate::api_types::ServerMsg;
use crate::server::WebState;

/// TTL của message push (giây) — thông báo của agent là tạm thời: confirm hết
/// hạn sau 300s, câu trả lời stale sau 1 phút không còn giá trị.
const TTL_SECONDS: u32 = 60;
/// Trần ký tự tiêu đề/nội dung push (cắt đúng ranh giới UTF-8).
const TITLE_MAX_CHARS: usize = 100;
const BODY_MAX_CHARS: usize = 300;
/// Timeout mỗi request gửi push — push service thường phản hồi < 1s.
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// Kết quả một lần gửi push tới một subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PushSendOutcome {
    /// Push service nhận (2xx).
    Delivered,
    /// Subscription không còn tồn tại (404/410) — phải xoá khỏi store.
    Gone,
    /// Lỗi khác (mạng, 429, 5xx...) — giữ subscription, đếm failure_count.
    Failed(String),
}

/// Nội dung một thông báo push, đã sẵn sàng mã hoá.
pub(crate) struct PushNotice {
    pub(crate) title: String,
    pub(crate) body: String,
    /// `high` cho confirm (cần hành động ngay), `normal` cho phần còn lại.
    pub(crate) urgency: &'static str,
}

/// Đường dẫn file khóa VAPID từ cấu hình (mở rộng `~` như `serve`).
pub fn load_vapid_keys(config: &Config) -> Result<VapidKeys, vapid::VapidError> {
    let data_dir = expand_tilde(&config.data.dir);
    VapidKeys::load_or_create(&vapid_keys_path(&data_dir))
}

/// Mở rộng `~/...` bằng `$HOME` — mẫu `chat::expand_tilde` (mục 18:
/// `data.dir = "~/.bean"`); bean-web không thấy crate `bean` nên tự giữ bản nhỏ.
fn expand_tilde(path: &std::path::Path) -> std::path::PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home);
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

/// Payload JSON push: `{title, body, url}` — `url` luôn là gốc app: bấm vào mở
/// Bean, UI tự khôi phục trạng thái qua `Sync` + REST (không deep-link tới
/// session cụ thể để tránh lệch route giữa các bản UI).
pub(crate) fn build_payload(notice: &PushNotice) -> Vec<u8> {
    serde_json::json!({
        "title": truncate_chars(&notice.title, TITLE_MAX_CHARS),
        "body": truncate_chars(&notice.body, BODY_MAX_CHARS),
        "url": "/",
    })
    .to_string()
    .into_bytes()
}

/// Cắt chuỗi theo **số ký tự** (không phải byte) — cắt theo byte tuỳ ý sẽ panic
/// giữa code point UTF-8 (pitfall 22.9).
pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// Sự kiện run → thông báo push. `None` = không đáng push (text delta, tool...).
fn notice_from_run_event(event: &RunEvent, state: &WebState) -> Option<PushNotice> {
    match event {
        RunEvent::Final { text, .. } => {
            let body = if text.trim().is_empty() {
                "Bean đã trả lời xong.".to_string()
            } else {
                text.clone()
            };
            Some(PushNotice {
                title: "Bean".to_string(),
                body,
                urgency: "normal",
            })
        }
        RunEvent::ConfirmRequest { prompt, role, .. } => {
            if !visible_to_admin(&state.config, role) {
                return None;
            }
            Some(PushNotice {
                title: "Bean cần xác nhận".to_string(),
                body: prompt.clone(),
                urgency: "high",
            })
        }
        _ => None,
    }
}

/// Sự kiện có thuộc role mà **admin** (người nhận duy nhất của push) được thấy
/// không — cùng ngữ nghĩa `ws.rs::event_visible_to` nhưng người xem cố định là
/// admin đầu tiên trong `agent.user_roles` (subscribe push là endpoint admin-only).
fn visible_to_admin(config: &Config, role: &Option<String>) -> bool {
    let Some(name) = role else {
        return true;
    };
    let Some((admin_user, _)) = config
        .agent
        .user_roles
        .iter()
        .find(|(_, mapped)| mapped.as_str() == "admin")
    else {
        // RBAC bật nhưng không có admin → không xác định được người nhận; hiếm
        // và vô hại: ứng dụng một người dùng, để sự kiện qua như ws.rs với
        // user ngoài map (permissions mặc định).
        return true;
    };
    let permissions = config.permissions_for(admin_user);
    config
        .role(name)
        .is_none_or(|role| permissions.can_view_agent(&role.tag_set()))
}

/// Vòng lặp dispatcher push: nghe sự kiện Router + notification, push khi
/// không có kết nối WS nào.
///
/// Chạy như một task của `serve`; dừng êm khi `shutdown` bị huỷ. Không bao giờ
/// panic: lỗi gửi được ghi log + đếm failure_count, lỗi store bị ghi log.
pub async fn run_push_dispatcher(state: WebState, shutdown: CancellationToken) {
    let keys = match load_vapid_keys(&state.config) {
        Ok(keys) => Arc::new(keys),
        Err(error) => {
            tracing::error!(%error, "không nạp được khóa VAPID — Web Push tắt phiên này");
            return;
        }
    };
    let client = match reqwest::Client::builder().timeout(SEND_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(%error, "không dựng được HTTP client — Web Push tắt phiên này");
            return;
        }
    };
    let mut events = state.router.events();
    let mut notifications = state.notifications.subscribe();
    tracing::info!("push dispatcher đã khởi động");

    loop {
        let notice = tokio::select! {
            () = shutdown.cancelled() => break,
            event = events.recv() => match event {
                Ok(event) => notice_from_run_event(&event, &state),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // Lagged = bỏ sót sự kiện; không tái phát lại (không có lịch
                    // sử sự kiện) — chỉ log, các sự kiện sau vẫn push bình thường.
                    tracing::warn!(skipped, "push dispatcher lagged, bỏ một số sự kiện");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            message = notifications.recv() => match message {
                Ok(ServerMsg::Notification { .. }) => Some(PushNotice {
                    title: "Bean".to_string(),
                    body: "Bạn có thông báo mới từ Bean.".to_string(),
                    urgency: "normal",
                }),
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "push dispatcher lagged kênh notification");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
        };
        let Some(notice) = notice else { continue };

        // Tab đang mở (WS còn kết nối) thì in-page notification đã đủ.
        if state.ws_active_count() > 0 {
            continue;
        }
        dispatch_to_all(&state, &client, &keys, &notice).await;
    }
    tracing::info!("push dispatcher đã dừng");
}

/// Gửi một thông báo tới mọi subscription; cập nhật trạng thái store theo kết quả.
pub(crate) async fn dispatch_to_all(
    state: &WebState,
    client: &reqwest::Client,
    keys: &VapidKeys,
    notice: &PushNotice,
) {
    let payload = build_payload(notice);
    let subscriptions = match state.store.list_push_subscriptions().await {
        Ok(list) => list,
        Err(error) => {
            tracing::error!(%error, "đọc subscription push thất bại");
            return;
        }
    };
    if subscriptions.is_empty() {
        return;
    }
    for subscription in &subscriptions {
        let outcome =
            send_to_subscription(client, keys, subscription, &payload, notice.urgency).await;
        match outcome {
            PushSendOutcome::Delivered => {
                record_outcome(state, &subscription.endpoint, true).await;
            }
            PushSendOutcome::Gone => {
                match state
                    .store
                    .delete_push_subscription(&subscription.endpoint)
                    .await
                {
                    Ok(true) => tracing::info!(
                        origin = endpoint_origin(&subscription.endpoint).as_str(),
                        "subscription push đã chết (404/410), đã xoá"
                    ),
                    Ok(false) => {}
                    Err(error) => tracing::warn!(%error, "xoá subscription chết thất bại"),
                }
            }
            PushSendOutcome::Failed(reason) => {
                tracing::warn!(
                    origin = endpoint_origin(&subscription.endpoint).as_str(),
                    %reason,
                    "gửi push thất bại"
                );
                record_outcome(state, &subscription.endpoint, false).await;
            }
        }
    }
}

/// Ghi kết quả gửi; lỗi store chỉ log — không được làm dispatcher chết vì DB.
async fn record_outcome(state: &WebState, endpoint: &str, ok: bool) {
    if let Err(error) = state.store.record_push_outcome(endpoint, ok).await {
        tracing::warn!(%error, ok, "ghi kết quả push thất bại");
    }
}

/// Gửi một payload (đã build) tới một subscription.
pub(crate) async fn send_to_subscription(
    client: &reqwest::Client,
    keys: &VapidKeys,
    subscription: &PushSubscription,
    payload: &[u8],
    urgency: &str,
) -> PushSendOutcome {
    let endpoint = Url::parse(&subscription.endpoint)
        .ok()
        .filter(|url| url.scheme() == "https");
    let Some(endpoint) = endpoint else {
        // Endpoint không https: chặn từ gốc (dữ liệu client không tin cậy) —
        // đăng ký ở `rest_push` cũng từ chối, đây là phòng thủ thứ hai.
        return PushSendOutcome::Failed("endpoint phải là https".into());
    };
    let aud = endpoint.origin().ascii_serialization();
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let authorization = match keys.authorization_header(&aud, now) {
        Ok(header) => header,
        Err(error) => return PushSendOutcome::Failed(error.to_string()),
    };
    let (ua_public, auth_secret) = match decode_subscription_keys(subscription) {
        Ok(pair) => pair,
        Err(error) => return PushSendOutcome::Failed(error.to_string()),
    };
    let body = match ece::encrypt(&ua_public, &auth_secret, payload) {
        Ok(body) => body,
        Err(error) => return PushSendOutcome::Failed(error.to_string()),
    };

    let response = client
        .post(endpoint.clone())
        .header("Authorization", authorization)
        .header("TTL", TTL_SECONDS.to_string())
        .header("Urgency", urgency)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .body(body)
        .send()
        .await;
    match response {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                PushSendOutcome::Delivered
            } else if status == reqwest::StatusCode::NOT_FOUND
                || status == reqwest::StatusCode::GONE
            {
                PushSendOutcome::Gone
            } else {
                PushSendOutcome::Failed(format!("HTTP {status}"))
            }
        }
        Err(error) => PushSendOutcome::Failed(error.to_string()),
    }
}

/// Giải khóa client (p256dh/auth) của subscription; lỗi → không gửi được.
fn decode_subscription_keys(subscription: &PushSubscription) -> Result<(Vec<u8>, Vec<u8>), String> {
    let ua_public = ece::decode_b64url(&subscription.p256dh)
        .map_err(|error| format!("p256dh không hợp lệ: {error}"))?;
    let auth_secret = ece::decode_b64url(&subscription.auth)
        .map_err(|error| format!("auth không hợp lệ: {error}"))?;
    Ok((ua_public, auth_secret))
}

/// Origin của endpoint để log an toàn — không log URL đầy đủ (chứa token
/// định danh subscription trong path).
pub(crate) fn endpoint_origin(endpoint: &str) -> String {
    Url::parse(endpoint)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| "<endpoint không hợp lệ>".into())
}

/// Băm endpoint thành định danh ngắn ổn định để log/UI hiển thị mà không lộ URL.
pub(crate) fn endpoint_label(endpoint: &str) -> String {
    let digest = sha2::Sha256::digest(endpoint.as_bytes());
    let mut short = [0u8; 6];
    short.copy_from_slice(&digest[..6]);
    format!("push-{}", B64URL.encode(short))
}

#[cfg(test)]
mod tests {
    // Cùng quy ước test với các file tests/*.rs của workspace.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn payload_has_title_body_url() {
        let payload = build_payload(&PushNotice {
            title: "Bean".into(),
            body: "Xong rồi".into(),
            urgency: "normal",
        });
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["title"], "Bean");
        assert_eq!(value["body"], "Xong rồi");
        assert_eq!(value["url"], "/");
    }

    /// Cắt nội dung quá dài ở ranh giới ký tự — với tiếng Việt có dấu và emoji
    /// không được panic (pitfall 22.9).
    #[test]
    fn truncate_handles_multibyte() {
        let text = "🦀".repeat(500);
        let cut = truncate_chars(&text, 300);
        assert_eq!(cut.chars().count(), 301); // 300 + dấu …
        let payload = build_payload(&PushNotice {
            title: "t".into(),
            body: text,
            urgency: "normal",
        });
        assert!(serde_json::from_slice::<serde_json::Value>(&payload).is_ok());
    }

    #[test]
    fn endpoint_label_is_stable_and_short() {
        let a = endpoint_label("https://fcm.googleapis.com/fcm/send/token-abc");
        let b = endpoint_label("https://fcm.googleapis.com/fcm/send/token-abc");
        assert_eq!(a, b);
        assert!(a.starts_with("push-"));
        assert!(a.len() < 16);
        assert_ne!(
            endpoint_label("https://fcm.googleapis.com/fcm/send/other"),
            a
        );
    }
}
