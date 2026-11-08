//! Handler REST cho toggle **Host mode** (D22.2): bật/tắt `run_shell` chạy trực
//! tiếp trên máy thay vì sandbox, từ UI — step-up bằng **mật khẩu tài khoản OS**
//! của tiến trình Bean (máy không đặt mật khẩu ⇒ bỏ qua).
//!
//! # Ranh giới bảo mật
//!
//! * `security.sandbox.mode` là khoá tầng B (M29) + `apply = Restart` — không
//!   hot-apply như LAN (D21.7): sandbox được dựng một lần lúc khởi động, hot-swap
//!   sẽ rời rạc giữa các tool đang chạy. UI hiển thị banner "cần restart" từ cặp
//!   `mode`/`running_mode`.
//! * No-op (mode đĩa đã bằng desired) → 200 `restart_required = false`, **không**
//!   đụng mật khẩu — tránh hỏi lại khi bấm Apply hai lần.
//! * Sai mật khẩu OS dùng chung bộ đếm khoá theo IP với step-up mật khẩu Bean
//!   (D19.5) — một bề mặt xác thực, một bộ đếm (`AuthService::os_step_up_*`).
//! * Inspect/verify là blocking (`su`, `passwd -S`, FFI Windows) — luôn
//!   `spawn_blocking` (mục 22.8).
//! * Lỗi hạ tầng xác minh (`passwd`/`su` không chạy được) ≠ sai mật khẩu: trả
//!   503 và **không** tính vào bộ đếm khoá — không khoá oan ai.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_security::OsAuthError;
use bean_settings::{SettingEntry, SettingValue};
use secrecy::SecretString;

use crate::api_types::{HostModeDto, HostModeMutateResponse, HostModeRequest};
use crate::server::rest_settings::{
    config_readonly, map_settings_error, peer_ip, require_admin, required_audit,
    unknown_setting_failure,
};
use crate::server::{ApiFailure, ApiResult, WebState, require_user};

/// Khoá mode sandbox trong settings registry.
const MODE_PATH: &str = "security.sandbox.mode";

/// Lỗi hạ tầng xác minh mật khẩu OS — 503, không phải lỗi người dùng.
fn os_auth_unavailable() -> ApiFailure {
    ApiFailure::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "os_auth_unavailable",
        "không xác minh được mật khẩu máy — hạ tầng xác minh của OS lỗi (`bean doctor` để kiểm tra)",
    )
}

/// Map lỗi mở phiên step-up: rate limit → 429 kèm số giây, còn lại 500.
fn map_begin_error(error: crate::auth::AuthError) -> ApiFailure {
    match error {
        crate::auth::AuthError::RateLimited(seconds) => ApiFailure::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("quá nhiều lần thử; thử lại sau {seconds} giây"),
        ),
        _ => ApiFailure::internal(),
    }
}

/// Mode ghi trên đĩa — nguồn sự thật, như `GET /api/settings`. Đĩa lỗi ⇒ fallback
/// snapshot khởi động (GET vẫn hiển thị gì đó, POST sẽ ghi lại đúng).
fn disk_mode(state: &WebState) -> String {
    let service = bean_settings::SettingsService::new(&state.config, state.router.config_path());
    bean_settings::lookup(MODE_PATH)
        .map(|entry| service.get(entry).value)
        .unwrap_or_else(|| state.config.security.sandbox.mode.as_str().to_string())
}

/// `GET /api/settings/host-mode` — mọi user đã đăng nhập, chỉ đọc: mode trên đĩa,
/// mode đang chạy, cần mật khẩu OS không (fail-closed khi không inspect được) và
/// cờ `pending` cho banner restart.
pub(super) async fn host_mode_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<HostModeDto>> {
    let _ = require_user(&state, &jar).await?;
    let running_mode = state.config.security.sandbox.mode.as_str().to_string();
    let mode = disk_mode(&state);
    let os_auth = state.os_auth.clone();
    // Blocking (`passwd -S` trên Linux) — spawn_blocking; mọi lỗi ⇒ `true` (fail-closed).
    let os_password_required = tokio::task::spawn_blocking(move || {
        os_auth
            .inspect()
            .map_or(true, |account| account.has_password)
    })
    .await
    .unwrap_or(true);
    Ok(Json(HostModeDto {
        pending: mode != running_mode,
        mode,
        running_mode,
        os_password_required,
    }))
}

/// `POST /api/settings/host-mode` — bật/tắt host mode: `enabled = true` ghi
/// `security.sandbox.mode = "host"`, `false` ghi `"docker"`. Admin + step-up mật
/// khẩu OS (khi máy có mật khẩu **và** request có ghi đĩa). Hiệu lực sau restart.
pub(super) async fn host_mode_update(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
    Json(request): Json<HostModeRequest>,
) -> ApiResult<Json<HostModeMutateResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let Some(config_path) = state.router.config_path() else {
        return Err(config_readonly());
    };
    let audit = required_audit(&state)?;
    let desired = if request.enabled { "host" } else { "docker" };

    // No-op trước step-up: bấm Apply khi đã ở đúng mode không nên hỏi mật khẩu.
    let entry: &'static SettingEntry =
        bean_settings::lookup(MODE_PATH).ok_or_else(|| unknown_setting_failure(MODE_PATH))?;
    let service = bean_settings::SettingsService::new(&state.config, Some(config_path));
    let current = service.get(entry).value;
    if current == desired {
        return Ok(Json(HostModeMutateResponse {
            mode: current,
            restart_required: false,
        }));
    }

    let ip = peer_ip(&peer);
    state
        .auth
        .os_step_up_begin(ip, Some(audit.as_ref()))
        .map_err(map_begin_error)?;

    // Inspect blocking — lỗi hạ tầng ⇒ 503, không đụng bộ đếm khoá.
    let os_auth = state.os_auth.clone();
    let account = tokio::task::spawn_blocking(move || os_auth.inspect())
        .await
        .map_err(|_| os_auth_unavailable())?
        .map_err(|_| os_auth_unavailable())?;

    if account.has_password {
        let Some(password) = request.os_password.as_deref() else {
            return Err(ApiFailure::new(
                StatusCode::UNAUTHORIZED,
                "os_password_required",
                "cần mật khẩu tài khoản máy để bật/tắt host mode",
            ));
        };
        let os_auth = state.os_auth.clone();
        let username = account.username.clone();
        let secret = SecretString::from(password.to_string());
        let verified = tokio::task::spawn_blocking(move || os_auth.verify(&username, &secret))
            .await
            .map_err(|_| os_auth_unavailable())?;
        match verified {
            Ok(()) => state.auth.os_step_up_finish(ip, true, Some(audit.as_ref())),
            Err(OsAuthError::InvalidPassword) => {
                state
                    .auth
                    .os_step_up_finish(ip, false, Some(audit.as_ref()));
                return Err(ApiFailure::new(
                    StatusCode::UNAUTHORIZED,
                    "invalid_os_password",
                    "mật khẩu máy không đúng",
                ));
            }
            Err(_) => return Err(os_auth_unavailable()),
        }
    }

    service
        .set_many(
            &[(entry, SettingValue::Str(desired.to_string()))],
            &audit,
            &user_id,
        )
        .map_err(map_settings_error)?;
    Ok(Json(HostModeMutateResponse {
        mode: desired.to_string(),
        restart_required: true,
    }))
}
