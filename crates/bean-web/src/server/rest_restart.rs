//! Phần "cần restart" của `GET /api/status` và `POST /api/system/restart` (M31).
//!
//! Nút restart kiểu Ubuntu: banner UI hiện khi có thứ chờ restart —
//!
//! * khoá cấu hình `apply = restart` đã ghi đĩa nhưng chưa hiệu lực (so giá trị
//!   đĩa với giá trị đang chạy, D21.7 — cùng phép so với cờ `pending` per-key
//!   của `GET /api/settings`; gồm cả host-mode/LAN khi hot-swap thất bại), và
//! * bản mới đã cài qua `bean update` nhưng chưa restart (marker
//!   `data.dir/update-pending.json`, M30/M31).
//!
//! Bấm "ngay" → tiến trình tắt êm rồi exec lại binary (xem [`super::restart`]);
//! "để sau" chỉ ẩn banner trong phiên phía client.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_security::entry_now;
use bean_settings::{SETTINGS, SettingApply};

use crate::api_types::{RestartResponse, RestartStatusDto};

use super::rest_settings::{require_admin, running_value};
use super::{ApiFailure, ApiResult, WebState, require_user};

/// Phần `restart` của `GET /api/status` — tổng hợp mọi nguồn chờ restart.
pub(super) fn restart_status(state: &WebState) -> RestartStatusDto {
    let mut reasons = Vec::new();
    if let Some(config_path) = state.router.config_path() {
        let service = bean_settings::SettingsService::new(&state.config, Some(config_path));
        let pending: Vec<&str> = SETTINGS
            .iter()
            .filter(|entry| entry.apply == SettingApply::Restart)
            .filter(|entry| service.get(entry).value != running_value(state, entry))
            .map(|entry| entry.path)
            .collect();
        if !pending.is_empty() {
            reasons.push(format!(
                "{} khoá cấu hình chưa hiệu lực: {}",
                pending.len(),
                pending.join(", ")
            ));
        }
    }
    if let Some(marker) = bean_update::read_pending_marker(&state.config.data.dir) {
        reasons.push(format!(
            "bản mới {} đã cài — khởi động lại để dùng",
            marker.tag
        ));
    }
    RestartStatusDto {
        required: !reasons.is_empty(),
        reasons,
    }
}

/// `POST /api/system/restart` — admin-only; từ chối khi còn run đang chạy.
///
/// Trả `202` rồi tiến trình tự tắt êm (đường SIGTERM của `serve.rs`) và exec
/// lại binary với argv gốc. WS của UI đứt và tự nối lại với backoff (cơ chế
/// sẵn có từ M10/M16); banner biến mất khi `/api/status` mới không còn
/// `restart.required`.
///
/// # Ranh giới bảo mật
///
/// * Guard Origin/Content-Type chuẩn của tầng `/api` áp sẵn.
/// * Admin-only (giống `set_tool_enabled`) — restart ảnh hưởng khả dụng nhưng
///   không lộ secret, không cần step-up mật khẩu (quyết định M31).
/// * Ghi audit `system_restart` kèm lý do (không chứa giá trị cấu hình).
pub(super) async fn system_restart(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<(StatusCode, Json<RestartResponse>)> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let running = state.router.snapshot().running.len();
    if running > 0 {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "run_active",
            format!("đang có {running} run chạy — bấm Dừng (hoặc /stop) rồi thử lại"),
        ));
    }
    if !state.restart.request() {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "restart_pending",
            "một restart đã được yêu cầu trước đó và chưa hoàn tất",
        ));
    }
    if let Some(audit) = state.audit.as_deref() {
        let mut entry = entry_now(
            0,
            "web",
            "system_restart",
            &serde_json::json!({ "reasons": restart_status(&state).reasons }),
        );
        entry.ok = Some(true);
        entry.decision = "allow";
        entry.decided_by = user_id.to_string();
        if let Err(error) = audit.record(&entry) {
            tracing::warn!(error = %error, "không ghi được audit system_restart");
        }
    }
    tracing::info!(actor = %user_id, "yêu cầu restart từ UI — tắt êm rồi exec lại binary");
    Ok((
        StatusCode::ACCEPTED,
        Json(RestartResponse { restarting: true }),
    ))
}
