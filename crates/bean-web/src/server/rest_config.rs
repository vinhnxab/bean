//! Handler REST: cấu hình runtime (mục 11.1 — mở rộng màn Cài đặt).
//!
//! # Ranh giới secret (mục 15.6)
//!
//! API key **không bao giờ** đi qua HTTP: GET trả cờ `key_configured` thay vì giá
//! trị, PUT chỉ đổi model trong danh sách `allowed_models` đã khai báo trong
//! `bean.toml`. Provider/base_url/API key đổi bằng cách sửa file cấu hình và
//! restart — v1 không hỗ trợ hot-swap provider vì phải rebuild client HTTP kèm
//! key mới, bề mặt lỗi lớn hơn giá trị mang lại.

use axum::Json;
use axum::extract::State;
use axum_extra::extract::cookie::CookieJar;

use crate::api_types::{LlmConfigResponse, LlmModelUpdateRequest, LlmModelUpdateResponse};

use super::{ApiFailure, ApiResult, WebState, require_user};

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
    Ok(Json(LlmConfigResponse {
        provider: llm.provider.as_str().to_string(),
        model: llm.model,
        allowed_models: llm.allowed_models,
        api_key_env: llm.api_key_env,
        base_url: llm.base_url,
        max_tokens: llm.max_tokens,
        key_configured: llm.key_configured,
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
