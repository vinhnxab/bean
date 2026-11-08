//! Handler REST cho kiểm tra bản mới `GET /api/update/check` (M30, D23).
//!
//! # Ranh giới
//!
//! * Chỉ **hỏi** GitHub Releases — không tải artifact, không ghi đĩa, không đụng
//!   binary. Self-update chỉ xảy ra qua CLI `bean update` (D23.6: người dùng chủ
//!   chốt "check tay, không phone-home"; UI không tự update ở v1).
//! * Admin-only khi RBAC bật (`require_admin`) — lượt gọi tốn một request mạng
//!   ra GitHub và lộ thông tin phiên bản, không thuộc quyền role thường.
//! * Lỗi mạng/API trả `502 update_check_failed` với message của `UpdateError`
//!   (không chứa secret — lỗi chỉ gồm URL GitHub/mã HTTP); nền tảng chưa được
//!   build trả `422 update_platform_unsupported` để UI hiện cảnh báo tải tay.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;

use crate::api_types::UpdateCheckDto;
use crate::server::rest_settings::require_admin;
use crate::server::{ApiFailure, ApiResult, WebState, require_user};

/// `GET /api/update/check` — hỏi GitHub bản stable mới nhất, so với phiên bản
/// binary đang chạy. GET không đổi dữ liệu nên không cần audit — cùng quy tắc
/// với các endpoint đọc khác.
pub(super) async fn update_check(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<UpdateCheckDto>> {
    let user_id = require_user(&state, &jar).await?;
    require_admin(&state, &user_id)?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let artifact = bean_update::platform_artifact().map(str::to_string);
    let Some(artifact) = artifact else {
        return Err(platform_unsupported());
    };
    let client = bean_update::http_client();
    let check = bean_update::check(&client, &current_version)
        .await
        .map_err(map_check_error)?;
    Ok(Json(UpdateCheckDto {
        current_version: check.current,
        latest_version: check.latest.tag,
        update_available: check.update_available,
        release_url: check.latest.url,
        notes: check.latest.notes,
        artifact: Some(artifact),
    }))
}

fn platform_unsupported() -> ApiFailure {
    ApiFailure::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "update_platform_unsupported",
        "nền tảng của server chưa có artifact trong release — hãy tải tay từ trang GitHub Releases",
    )
}

fn map_check_error(error: bean_update::UpdateError) -> ApiFailure {
    match error {
        bean_update::UpdateError::NoStableRelease { .. } => ApiFailure::new(
            StatusCode::NOT_FOUND,
            "no_stable_release",
            "chưa có bản stable nào được phát hành",
        ),
        other => ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "update_check_failed",
            other.to_string(),
        ),
    }
}
