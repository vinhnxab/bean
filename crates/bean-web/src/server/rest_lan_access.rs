//! Handler REST cho tùy chọn **LAN Access** (D21.x): bật/tắt truy cập Bean từ
//! mạng nội bộ ngay trên UI thay vì sửa tay `bean.toml`.
//!
//! # Ranh giới bảo mật
//!
//! Ba khoá bị sửa (`web.allow_remote`, `web.bind`, `web.public_origin`) đều là
//! tầng B (M29) — đây là **ngoại lệ có kiểm soát** do người chủ dự án yêu cầu
//! (D21.x): chỉ admin khi RBAC bật, **step-up mật khẩu** như secrets (D19.5),
//! audit một event `config.set` cho mỗi khoá, và hiệu lực phải sau
//! `scripts/beanctl restart` (UI tái dụng banner pending restart).
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
use crate::server::{ApiFailure, ApiResult, WebState, require_user};

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

/// `GET /api/settings/lan-access` — mọi user đã đăng nhập, chỉ đọc: chế độ hiện
/// hành, IP LAN dò được, cổng và URL tương ứng.
pub(super) async fn lan_access_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<LanAccessDto>> {
    let _ = require_user(&state, &jar).await?;
    Ok(Json(lan_access_dto(
        state.config.web.bind,
        state.config.web.allow_remote,
        detect_lan_ipv4(),
    )))
}

/// `POST /api/settings/lan-access` — bật/tắt truy cập LAN (D21.x): ghi **atomic**
/// ba khoá tầng B trong một lượt validate qua `SettingsService::set_many`.
/// Admin + step-up mật khẩu + audit từng khoá; hiệu lực sau `beanctl restart`.
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
    let port = state.config.web.bind.port();

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
    Ok(Json(LanAccessMutateResponse {
        access: lan_access_dto(bind, allow_remote, lan_ip),
        restart_required: true,
    }))
}
