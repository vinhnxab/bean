//! Handler REST cho tùy chọn **LAN Access** (D21.x): bật/tắt truy cập Bean từ
//! mạng nội bộ ngay trên UI thay vì sửa tay `bean.toml`.
//!
//! # Ranh giới bảo mật
//!
//! Ba khoá bị sửa (`web.allow_remote`, `web.bind`, `web.public_origin`) đều là
//! tầng B (M29) — đây là **ngoại lệ có kiểm soát** do người chủ dự án yêu cầu
//! (D21.x): chỉ admin khi RBAC bật, **step-up mật khẩu** như secrets (D19.5),
//! audit một event `config.set` cho mỗi khoá, và hiệu lực **lúc nóng** qua hot
//! rebind (D21.7) — restart chỉ cần khi supervisor không swap được listener.
//!
//! # Vì sao ghi atomic cả ba khoá
//!
//! `Config::validate()` từ chối `bind` ngoài loopback khi `allow_remote = false`,
//! nên chuyển đổi bằng từng khoá riêng lẻ đều rơi vào trạng thái nửa chừng bị
//! từ chối. `SettingsService::set_many` ghi cả ba trong một lượt validate — toàn
//! bộ hoặc không khoá nào.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use axum::Json;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_settings::{SettingEntry, SettingValue};

use crate::api_types::{LanAccessDto, LanAccessMutateResponse, LanAccessRequest};
use crate::server::rest_settings::{
    config_readonly, map_settings_error, peer_ip, require_admin, required_audit, step_up,
    unknown_setting_failure,
};
use crate::server::{ApiFailure, ApiResult, WebListen, WebState, require_user};

/// Dò IPv4 LAN của máy server: trick UDP connect — không gửi packet nào, chỉ để
/// kernel chọn đường ra rồi đọc địa chỉ local của socket. Thử lần lượt hai địa
/// chỉ công cộng, bỏ qua loopback. `None` khi máy không có đường ra (máy không
/// có LAN thì không có gì để mở).
fn detect_lan_ipv4() -> Option<Ipv4Addr> {
    for probe in ["8.8.8.8:80", "1.1.1.1:443"] {
        let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
            continue;
        };
        if socket.connect(probe).is_err() {
            continue;
        }
        let Ok(local) = socket.local_addr() else {
            continue;
        };
        if let IpAddr::V4(ip) = local.ip()
            && !ip.is_loopback()
        {
            return Some(ip);
        }
    }
    None
}

/// DTO trạng thái LAN access từ bind + cờ allow_remote + IP dò được.
fn lan_access_dto(bind: SocketAddr, allow_remote: bool, lan_ip: Option<Ipv4Addr>) -> LanAccessDto {
    let mode = if allow_remote && !bind.ip().is_loopback() {
        "lan"
    } else {
        "loopback"
    };
    LanAccessDto {
        mode: mode.to_string(),
        lan_ip: lan_ip.map(|ip| ip.to_string()),
        port: bind.port(),
        url: (mode == "lan").then(|| {
            format!(
                "http://{}:{}",
                lan_ip.map_or_else(|| bind.ip(), IpAddr::from),
                bind.port()
            )
        }),
    }
}

/// Trạng thái LAN **sẽ hiệu lực** đọc qua `SettingsService` (nguồn sự thật trên
/// đĩa, như `GET /api/settings`): sau khi POST ghi file, GET phải phản ánh trạng
/// thái mới — không phải snapshot in-memory lúc khởi động (lỗi thật 2026-11-08).
/// Đĩa lỗi/parse lỗi ⇒ fallback cell nóng [`WebState::web_listen`].
fn effective_lan_view(state: &WebState) -> (SocketAddr, bool) {
    let service = bean_settings::SettingsService::new(&state.config, state.router.config_path());
    let fallback = state.web_listen.borrow().clone();
    let bind = bean_settings::lookup("web.bind")
        .map(|entry| service.get(entry).value)
        .and_then(|value| value.parse::<SocketAddr>().ok())
        .unwrap_or(fallback.bind);
    let allow_remote = bean_settings::lookup("web.allow_remote")
        .map(|entry| service.get(entry).value == "true")
        .unwrap_or(fallback.allow_remote);
    (bind, allow_remote)
}

/// `GET /api/settings/lan-access` — mọi user đã đăng nhập, chỉ đọc: trạng thái
/// hiệu lực (từ `bean.toml` trên đĩa, khớp cell nóng sau rebind), IP LAN dò được,
/// cổng và URL.
pub(super) async fn lan_access_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<LanAccessDto>> {
    let _ = require_user(&state, &jar).await?;
    let (bind, allow_remote) = effective_lan_view(&state);
    Ok(Json(lan_access_dto(bind, allow_remote, detect_lan_ipv4())))
}

/// `POST /api/settings/lan-access` — bật/tắt truy cập LAN (D21.x): ghi **atomic**
/// ba khoá tầng B trong một lượt validate qua `SettingsService::set_many`.
/// Admin + step-up mật khẩu + audit từng khoá. Sau khi ghi, áp dụng **lúc nóng**
/// qua `rebind` (D21.7): `restart_required = false` khi supervisor đã swap listener
/// thành công; `true` khi bind mới thất bại (listener cũ được khôi phục, cấu hình
/// đã ghi đĩa sẽ hiệu lực sau restart).
pub(super) async fn lan_access_update(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
    Json(request): Json<LanAccessRequest>,
) -> ApiResult<Json<LanAccessMutateResponse>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    step_up(&state, &request.password, peer_ip(&peer))?;
    let Some(config_path) = state.router.config_path() else {
        return Err(config_readonly());
    };
    let audit = required_audit(&state)?;
    let port = state.web_bind().port();

    let (bind, allow_remote, lan_ip) = if request.enabled {
        let Some(ip) = detect_lan_ipv4() else {
            return Err(ApiFailure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "lan_ip_unavailable",
                "Không dò được IP LAN của máy server — không thể mở bind ngoài loopback",
            ));
        };
        (
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
            true,
            Some(ip),
        )
    } else {
        (
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            false,
            detect_lan_ipv4(),
        )
    };
    let origin_value = if let (true, Some(ip)) = (request.enabled, lan_ip) {
        // Bật LAN: origin theo IP dò được. Tắt LAN (hoặc bất thường không có IP):
        // luôn về loopback — không bao giờ giữ IP LAN trong public_origin khi đã đóng.
        format!("http://{ip}:{port}")
    } else {
        format!("http://127.0.0.1:{port}")
    };
    // D21.7: validate TRƯỚC khi ghi đĩa — origin hỏng không bao giờ được persist.
    let public_origin = match super::rebind::validate_origin(&origin_value) {
        Ok(url) => url,
        Err(_) => {
            return Err(ApiFailure::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_origin",
                format!("origin không hợp lệ: {origin_value}"),
            ));
        }
    };
    let entry = |path: &str| -> Result<&'static SettingEntry, ApiFailure> {
        bean_settings::lookup(path).ok_or_else(|| unknown_setting_failure(path))
    };
    let allow_remote_entry = entry("web.allow_remote")?;
    let bind_entry = entry("web.bind")?;
    let origin_entry = entry("web.public_origin")?;
    let service = bean_settings::SettingsService::new(&state.config, Some(config_path));
    service
        .set_many(
            &[
                (allow_remote_entry, SettingValue::Bool(allow_remote)),
                (bind_entry, SettingValue::Str(bind.to_string())),
                (origin_entry, SettingValue::Str(origin_value)),
            ],
            &audit,
            &user_id,
        )
        .map_err(map_settings_error)?;
    // D21.7: áp dụng lúc nóng — guard đọc origin mới từ cell nóng ngay sau swap.
    let listen = WebListen {
        bind,
        allow_remote,
        public_origin,
    };
    let restart_required = state.rebind.apply(listen).await.is_err();
    Ok(Json(LanAccessMutateResponse {
        access: lan_access_dto(bind, allow_remote, lan_ip),
        restart_required,
    }))
}
