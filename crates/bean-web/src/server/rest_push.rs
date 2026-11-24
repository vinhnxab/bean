//! Handler REST cho Web Push (VAPID): trạng thái, đăng ký, gỡ, gửi thử.
//!
//! # Bất biến
//!
//! * Mọi handler đều `require_user`; subscribe/unsubscribe/test còn cần **admin**
//!   (cùng quy tắc `rest_voice.rs`) — đăng ký nhận push là thay đổi bề mặt tiếp
//!   cận của agent, một trình duyệt lạ không được phép tự rắc thông báo.
//! * Endpoint do client gửi là dữ liệu **không tin cậy**: chỉ nhận `https`,
//!   độ dài khóa kiểm trước khi lưu; URL nguyên văn không bao giờ vào log hay
//!   response (chỉ trả `label` = hash ngắn).
//! * Khóa VAPID nạp/sinh qua `push::load_vapid_keys` — lần đầu sinh file
//!   `data.dir/push-vapid.json` 0600; lỗi trả `503 push_unavailable`, không âm thầm
//!   trả khóa rỗng.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_memory::NewPushSubscription;

use crate::api_types::{
    PushKeyResponse, PushStatusResponse, PushSubscribeRequest, PushSubscriptionDto,
    PushTestResponse, PushUnsubscribeRequest,
};
use crate::push::{self, PushSendOutcome};

use super::{ApiFailure, ApiResult, WebState, require_user};

/// Trần User-Agent lưu vào store — chỉ để hiển thị, không cần nguyên bản dài.
const USER_AGENT_MAX_CHARS: usize = 200;

/// Guard admin dùng chung cho các endpoint push thay đổi dữ liệu/danh sách.
fn require_admin(state: &WebState, user_id: &str) -> Result<(), ApiFailure> {
    let permissions = state.config.permissions_for(user_id);
    if state.config.rbac_enabled() && permissions.role != "admin" {
        return Err(ApiFailure::new(
            StatusCode::FORBIDDEN,
            "admin_required",
            "chỉ admin được cấu hình Web Push",
        ));
    }
    Ok(())
}

/// Nạp khóa VAPID (sinh lần đầu nếu thiếu); lỗi → 503 với mã rõ ràng.
fn vapid_keys(state: &WebState) -> Result<push::VapidKeys, ApiFailure> {
    push::load_vapid_keys(&state.config).map_err(|error| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "push_unavailable",
            format!("không nạp được khóa VAPID: {error}"),
        )
    })
}

/// `GET /api/push` — trạng thái + danh sách subscription (admin).
pub(super) async fn push_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<PushStatusResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let (configured, public_key) = match vapid_keys(&state) {
        Ok(keys) => (true, keys.public_key_b64()),
        // Lỗi khóa không làm endpoint chết — UI cần biết trạng thái để tắt nút.
        Err(_) => (false, String::new()),
    };
    let subscriptions = state
        .store
        .list_push_subscriptions()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .map(|sub| PushSubscriptionDto {
            label: crate::push::endpoint_label(&sub.endpoint),
            user_agent: sub.user_agent,
            created_at: sub.created_at,
            failure_count: sub.failure_count,
        })
        .collect();
    Ok(Json(PushStatusResponse {
        configured,
        public_key,
        subscriptions,
    }))
}

/// `GET /api/push/key` — public key VAPID cho `pushManager.subscribe`.
///
/// Chỉ `require_user` (không admin): public key không phải secret, và UI có thể
/// cần key trước khi biết role. Đăng ký (`subscribe`) mới là bước cần admin.
pub(super) async fn push_key(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<PushKeyResponse>> {
    let _ = require_user(&state, &jar).await?;
    let keys = vapid_keys(&state)?;
    Ok(Json(PushKeyResponse {
        public_key: keys.public_key_b64(),
    }))
}

/// `POST /api/push/subscribe` — lưu subscription của trình duyệt hiện tại.
pub(super) async fn push_subscribe(
    State(state): State<WebState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<PushSubscribeRequest>,
) -> ApiResult<StatusCode> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    validate_subscription(&request)?;
    // Khóa VAPID phải nạp được trước khi lưu — không lưu subscription mà server
    // không gửi được gì (lỗi khóa sẽ hiện ở GET /api/push).
    let _keys = vapid_keys(&state)?;
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .map(|value| crate::push::truncate_chars(value, USER_AGENT_MAX_CHARS))
        .unwrap_or_default();
    let record = NewPushSubscription {
        endpoint: request.endpoint,
        p256dh: request.keys.p256dh,
        auth: request.keys.auth,
        user_agent,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    state
        .store
        .upsert_push_subscription(&record)
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/push/unsubscribe` — gỡ subscription (trình duyệt gọi khi
/// `pushSubscription.unsubscribe()` hoặc khi người dùng tắt thủ công).
pub(super) async fn push_unsubscribe(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<PushUnsubscribeRequest>,
) -> ApiResult<StatusCode> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let removed = state
        .store
        .delete_push_subscription(&request.endpoint)
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !removed {
        return Err(ApiFailure::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/push/test` — gửi thông báo thử tới mọi subscription và báo số
/// kết quả để UI xác nhận ngay "điện thoại có nhận không".
pub(super) async fn push_test(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<PushTestResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let keys = vapid_keys(&state)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| ApiFailure::internal())?;
    let notice = crate::push::PushNotice {
        title: "Bean".into(),
        body: "Đây là thông báo thử từ Bean — mọi thứ hoạt động bình thường.".into(),
        urgency: "normal",
    };
    let payload = crate::push::build_payload(&notice);
    let subscriptions = state
        .store
        .list_push_subscriptions()
        .await
        .map_err(|_| ApiFailure::internal())?;
    let mut response = PushTestResponse {
        delivered: 0,
        failed: 0,
        removed: 0,
    };
    for subscription in &subscriptions {
        let outcome = crate::push::send_to_subscription(
            &client,
            &keys,
            subscription,
            &payload,
            notice.urgency,
        )
        .await;
        match outcome {
            PushSendOutcome::Delivered => {
                response.delivered += 1;
                let _ = state
                    .store
                    .record_push_outcome(&subscription.endpoint, true)
                    .await;
            }
            PushSendOutcome::Gone => {
                response.removed += 1;
                let _ = state
                    .store
                    .delete_push_subscription(&subscription.endpoint)
                    .await;
            }
            PushSendOutcome::Failed(reason) => {
                response.failed += 1;
                tracing::warn!(
                    origin = crate::push::endpoint_origin(&subscription.endpoint),
                    %reason,
                    "push thử nghiệm thất bại"
                );
                let _ = state
                    .store
                    .record_push_outcome(&subscription.endpoint, false)
                    .await;
            }
        }
    }
    Ok(Json(response))
}

/// Kiểm tra subscription từ client trước khi lưu — fail-closed.
fn validate_subscription(request: &PushSubscribeRequest) -> Result<(), ApiFailure> {
    let endpoint = url::Url::parse(&request.endpoint).map_err(|_| {
        ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_endpoint",
            "endpoint không phải URL hợp lệ",
        )
    })?;
    // Chỉ https: push service của trình duyệt luôn https; chặn http chặn luôn
    // SSRF vào nội bộ (mục 15.5) từ dữ liệu không tin cậy.
    if endpoint.scheme() != "https" {
        return Err(ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_endpoint",
            "endpoint push phải là https",
        ));
    }
    if request.endpoint.len() > 2048 {
        return Err(ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_endpoint",
            "endpoint quá dài",
        ));
    }
    // p256dh phải là public key P-256 hợp lệ (65 byte uncompressed) — kiểm ở đây
    // để lỗi báo về trình duyệt ngay, không đợi lần gửi đầu tiên thất bại.
    let ua_public = crate::push::decode_client_b64(&request.keys.p256dh).map_err(|_| {
        ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_keys",
            "p256dh không phải base64url hợp lệ",
        )
    })?;
    if ua_public.len() != 65 || ua_public[0] != 0x04 {
        return Err(ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_keys",
            "p256dh phải là public key P-256 uncompressed 65 byte",
        ));
    }
    let auth = crate::push::decode_client_b64(&request.keys.auth).map_err(|_| {
        ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_keys",
            "auth không phải base64url hợp lệ",
        )
    })?;
    if auth.is_empty() || auth.len() > 255 {
        return Err(ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_keys",
            "auth secret phải 16 byte theo RFC 8291",
        ));
    }
    Ok(())
}
