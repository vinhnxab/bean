//! Handler REST: cấu hình runtime (mục 11.1 — mở rộng màn Cài đặt).
//!
//! # Ranh giới secret (mục 15.6)
//!
//! API key **không bao giờ** đi qua HTTP: GET trả cờ `key_configured` thay vì giá
//! trị, PUT chỉ đổi model trong danh sách `allowed_models` đã khai báo trong
//! `bean.toml`. Hot-swap provider (PUT `/api/config/llm/provider`) nhận **tên**
//! profile đã khai báo sẵn trong `[[llm.providers]]` — key mới được resolve từ
//! biến môi trường phía server, không bao giờ nhận qua body.

use axum::Json;
use axum::extract::State;
use axum_extra::extract::cookie::CookieJar;

use crate::api_types::{
    LlmConfigResponse, LlmModelUpdateRequest, LlmModelUpdateResponse, LlmProviderInfo,
    LlmProviderSwitchRequest,
};

use super::{ApiFailure, ApiResult, WebState, require_user};

/// Chuyển [`bean_core::LlmProfileInfo`] (core, không serde) sang DTO web.
fn provider_info(profile: bean_core::LlmProfileInfo) -> LlmProviderInfo {
    LlmProviderInfo {
        name: profile.name,
        provider: profile.provider.as_str().to_string(),
        model: profile.model,
        allowed_models: profile.allowed_models,
        base_url: profile.base_url,
        api_key_env: profile.api_key_env,
        key_configured: profile.key_configured,
        active: profile.active,
    }
}

/// `GET /api/config/llm` — cấu hình LLM runtime, đã khử secret.
pub(super) async fn get_llm_config(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<LlmConfigResponse>> {
    let _ = require_user(&state, &jar).await?;
    let llm = state
        .router
        .llm_config()
        .map_err(|_| ApiFailure::internal())?;
    let providers = state
        .router
        .llm_profiles()
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .map(provider_info)
        .collect();
    Ok(Json(LlmConfigResponse {
        provider: llm.provider.as_str().to_string(),
        model: llm.model,
        allowed_models: llm.allowed_models,
        api_key_env: llm.api_key_env,
        base_url: llm.base_url,
        max_tokens: llm.max_tokens,
        key_configured: llm.key_configured,
        providers,
    }))
}

/// `PUT /api/config/llm/model` — đổi model runtime (cùng nguồn với `/model`).
pub(super) async fn update_llm_model(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<LlmModelUpdateRequest>,
) -> ApiResult<Json<LlmModelUpdateResponse>> {
    let _ = require_user(&state, &jar).await?;
    let model = request.model.trim();
    if model.is_empty() {
        return Err(ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "model_required",
            "model không được rỗng",
        ));
    }
    state
        .router
        .set_llm_model(model)
        .map_err(|error| match error {
            bean_core::RouterError::ModelNotAllowed(_) => ApiFailure::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "model_not_allowed",
                "model không nằm trong llm.allowed_models",
            ),
            _ => ApiFailure::internal(),
        })?;
    Ok(Json(LlmModelUpdateResponse {
        model: model.to_string(),
    }))
}

/// `PUT /api/config/llm/provider` — hot-swap provider theo profile đã khai báo.
///
/// Trả snapshot `LlmConfigResponse` SAU khi switch để UI cập nhật cache một lần.
pub(super) async fn update_llm_provider(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<LlmProviderSwitchRequest>,
) -> ApiResult<Json<LlmConfigResponse>> {
    let _ = require_user(&state, &jar).await?;
    let profile = request.profile.trim();
    if profile.is_empty() {
        return Err(ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "profile_required",
            "profile không được rỗng",
        ));
    }
    let model = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let snapshot =
        state
            .router
            .switch_llm_provider(profile, model)
            .map_err(|error| match error {
                bean_core::RouterError::ProfileNotFound(_) => ApiFailure::new(
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "profile_not_found",
                    "profile không tồn tại trong cấu hình",
                ),
                bean_core::RouterError::ModelNotAllowed(_) => ApiFailure::new(
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "model_not_allowed",
                    "model không nằm trong allowed_models của profile",
                ),
                bean_core::RouterError::LlmProvider(_) => ApiFailure::new(
                    axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                    "provider_build_failed",
                    "không dựng được provider — kiểm tra biến môi trường API key",
                ),
                _ => ApiFailure::internal(),
            })?;
    let providers = state
        .router
        .llm_profiles()
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .map(provider_info)
        .collect();
    Ok(Json(LlmConfigResponse {
        provider: snapshot.provider.as_str().to_string(),
        model: snapshot.model,
        allowed_models: snapshot.allowed_models,
        api_key_env: snapshot.api_key_env,
        base_url: snapshot.base_url,
        max_tokens: snapshot.max_tokens,
        key_configured: snapshot.key_configured,
        providers,
    }))
}
