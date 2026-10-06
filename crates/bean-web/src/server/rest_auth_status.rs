//! Handler REST: dang nhap, dang xuat, trang thai va danh sach agent.
//!
//! # Loi khong duoc pham
//!
//! Token phiendang khong duoc tra trong JSON — chi dat cookie. `login` lay IP that tu
//! `ConnectInfo` (khong tin header `X-Forwarded-For`) de gioi han tan suat theo IP that.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::Json;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Extension, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;

use crate::api_types::{
    AgentListResponse, AgentRelationDto, AgentReportDto, AgentStatusDto, AuthMeResponse,
    LoginRequest, LoginResponse, LogoutResponse, StatusResponse,
};

use crate::auth::AuthError;

use super::{ApiFailure, ApiResult, WebState, cookie_token, require_user};

/// Đăng nhập; không trả token trong JSON, chỉ đặt cookie.
pub(super) async fn login(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiFailure> {
    let ip = peer.map_or(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        |Extension(ConnectInfo(peer))| peer.ip(),
    );
    let session = state
        .auth
        .login(&request.password, ip, state.audit.as_deref())
        .await
        .map_err(|error| match error {
            AuthError::InvalidCredentials => ApiFailure::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "thông tin đăng nhập không đúng",
            ),
            AuthError::RateLimited(seconds) => {
                let mut failure = ApiFailure::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "quá nhiều lần thử; thử lại sau",
                );
                failure.message = format!("quá nhiều lần thử; thử lại sau {seconds} giây");
                failure
            }
            _ => ApiFailure::internal(),
        })?;
    let mut response = Json(LoginResponse {
        user_id: state.config.web.user_id.clone(),
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.auth.session_cookie(&session))
            .map_err(|_| ApiFailure::internal())?,
    );
    Ok(response)
}

/// Đăng xuất và xoá bản ghi server-side.
pub(super) async fn logout(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
) -> Result<Response, ApiFailure> {
    if let Some(token) = cookie_token(&jar) {
        let ip = peer.map_or(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            |Extension(ConnectInfo(peer))| peer.ip(),
        );
        state
            .auth
            .logout(&token, state.audit.as_deref(), ip)
            .await
            .map_err(|_| ApiFailure::internal())?;
    }
    let mut response = Json(LogoutResponse { ok: true }).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.auth.expired_cookie()).map_err(|_| ApiFailure::internal())?,
    );
    Ok(response)
}

pub(super) async fn auth_me(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<AuthMeResponse>> {
    let user_id = require_user(&state, &jar).await?;
    Ok(Json(AuthMeResponse { user_id }))
}

/// Trạng thái runtime cho API.
pub(super) async fn status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<StatusResponse>> {
    let _ = require_user(&state, &jar).await?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let usage = state
        .store
        .usage(&today)
        .await
        .map_err(|_| ApiFailure::internal())?;
    let mut channels = Vec::with_capacity(2);
    if state.config.web.enabled {
        channels.push("web".to_string());
    }
    if state.config.telegram.enabled {
        channels.push("telegram".to_string());
    }
    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        model: state.config.llm.model.clone(),
        max_steps: state.config.agent.max_steps,
        daily_token_budget: state.config.security.daily_token_budget,
        tokens_used: u64::from(usage.total()),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        channels,
    }))
}

/// Danh sách agent cho HUB, **đã lọc theo RBAC trước khi tuần tự hoá**.
///
/// # Vì sao lọc ở đây chứ không ở client
///
/// Ràng buộc bắt buộc: role không có quyền phải **không nhận** dữ liệu đó trong
/// response, chứ không phải "UI tự ẩn đi". Ẩn bằng CSS/JS vẫn để nguyên payload
/// trong tab Network — đó là rò dữ liệu, không phải phân quyền.
///
/// Vì vậy `filter` chạy **trước** khi `AgentReportDto` được dựng: agent bị chặn
/// không bao giờ tồn tại trong response, và tên role của nó cũng không lộ ra.
///
/// Quyết định dùng [`RolePermissions::can_view_agent`] — cùng hệ tag RBAC, tách
/// khỏi `allows` vì đây là câu hỏi *quan sát*, không phải *quyền hành động*.
/// khỏi `allows` vì đây là câu hỏi *quan sát*, không phải *quyền hành động*.
pub(super) async fn list_agents(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<AgentListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let permissions = state.config.permissions_for(&user_id);
    let agents = state
        .router
        .agent_reports()
        .into_iter()
        .filter(|report| {
            state
                .config
                .role(&report.role)
                .is_some_and(|role| permissions.can_view_agent(&role.tag_set()))
        })
        .map(|report| AgentReportDto {
            role: report.role,
            status: match report.status {
                bean_types::AgentStatus::Idle => AgentStatusDto::Idle,
                bean_types::AgentStatus::Working => AgentStatusDto::Working,
                bean_types::AgentStatus::AwaitingYou => AgentStatusDto::AwaitingYou,
            },
            summary: report.summary,
            risks: report.risks,
            relation: match report.relation {
                bean_types::AgentRelation::Manages => AgentRelationDto::Manages,
                bean_types::AgentRelation::Reviews => AgentRelationDto::Reviews,
                bean_types::AgentRelation::AlertsDirectly => AgentRelationDto::AlertsDirectly,
            },
        })
        .collect();
    Ok(Json(AgentListResponse {
        agents,
        viewer_role: permissions.role,
    }))
}
