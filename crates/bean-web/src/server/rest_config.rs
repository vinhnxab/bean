//! Handler REST: cấu hình runtime (mục 11.1 — mở rộng màn Cài đặt).
//!
//! # Ranh giới secret (mục 15.6)
//!
//! API key là **ghi một chiều** qua HTTP: GET trả cờ `key_configured` + hint 4
//! ký tự cuối thay vì giá trị; PUT/POST nhận key mới qua body đúng một lần để
//! lưu vào `bean.toml` (file tự chmod 0600), không bao giờ trả key nguyên vẹn.
//! Hot-swap provider (PUT `/api/config/llm/provider`) nhận **tên** profile đã
//! khai báo sẵn trong `[[llm.providers]]` — key mới được resolve phía server
//! (file key trước, env sau), không nhận qua body.

use axum::Json;
use axum::extract::{Path, State};
use axum_extra::extract::cookie::CookieJar;
use bean_types::config::{LlmProfile, LlmProviderKind};

use crate::api_types::{
    LlmConfigResponse, LlmModelUpdateRequest, LlmModelUpdateResponse, LlmProviderCreateRequest,
    LlmProviderInfo, LlmProviderSwitchRequest, LlmProviderUpdateRequest,
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
        max_tokens: profile.max_tokens,
        api_key_env: profile.api_key_env,
        key_configured: profile.key_configured,
        api_key_hint: profile.api_key_hint,
        active: profile.active,
    }
}

/// Gate chung cho CRUD profile: `bean serve` phải biết đường dẫn `bean.toml` gốc
/// thì ghi vào `[[llm.providers]]` mới có ý nghĩa; CLI/test không bật.
fn ensure_profiles_writable(state: &WebState) -> Result<(), ApiFailure> {
    if !state.router.llm_profiles_writable() {
        return Err(ApiFailure::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "config_readonly",
            "server không biết đường dẫn bean.toml — không thể thêm/sửa/xoá profile",
        ));
    }
    Ok(())
}

/// Dựng snapshot `LlmConfigResponse` từ trạng thái hiện tại của Router (khử secret).
fn llm_config_response(state: &WebState) -> Result<LlmConfigResponse, ApiFailure> {
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
    Ok(LlmConfigResponse {
        provider: llm.provider.as_str().to_string(),
        model: llm.model,
        allowed_models: llm.allowed_models,
        api_key_env: llm.api_key_env,
        base_url: llm.base_url,
        max_tokens: llm.max_tokens,
        key_configured: llm.key_configured,
        providers,
    })
}

fn parse_provider_kind(raw: &str) -> Result<LlmProviderKind, ApiFailure> {
    match raw {
        "anthropic" => Ok(LlmProviderKind::Anthropic),
        "openai_compat" => Ok(LlmProviderKind::OpenAiCompat),
        _ => Err(ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_provider",
            "provider phải là 'anthropic' hoặc 'openai_compat'",
        )),
    }
}

/// Map [`bean_core::RouterError`] của CRUD profile sang HTTP failure.
fn map_profile_error(error: bean_core::RouterError) -> ApiFailure {
    use bean_core::RouterError;
    match error {
        RouterError::ProfileNotFound(_) => ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "profile_not_found",
            "profile không tồn tại trong cấu hình",
        ),
        RouterError::ProfileExists(_) => ApiFailure::new(
            axum::http::StatusCode::CONFLICT,
            "profile_exists",
            "đã có profile với tên này",
        ),
        RouterError::ProfileActive(_) => ApiFailure::new(
            axum::http::StatusCode::CONFLICT,
            "profile_active",
            "profile đang active — hãy chuyển sang profile khác trước",
        ),
        RouterError::DefaultProfileReadonly => ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "default_readonly",
            "profile 'default' là ánh xạ của [llm] — không sửa/xoá được",
        ),
        RouterError::ProfileInvalid(detail) => ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "profile_invalid",
            detail,
        ),
        RouterError::LlmProvider(_) => ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "provider_build_failed",
            "không dựng được provider — kiểm tra biến môi trường API key",
        ),
        RouterError::Persist(_) => ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "llm_state_unwritable",
            "không ghi được llm-state.json — kiểm tra quyền ghi data.dir",
        ),
        RouterError::ConfigPersist(_) => ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "config_unwritable",
            "không ghi được bean.toml — kiểm tra quyền ghi file cấu hình",
        ),
        _ => ApiFailure::internal(),
    }
}

/// Dựng ([`bean_types::config::LlmProfile`], [`bean_core::ApiKeyUpdate`]) từ
/// request sửa. Trim mọi trường chuỗi trước khi dựng — validate_for_storage kiểm
/// tra chuỗi nguyên bản nên phải làm sạch ở biên giới HTTP.
///
/// `api_key`: `None` ⇒ Keep, `Some("")` ⇒ Clear, `Some(k)` ⇒ Set.
fn parse_update_profile(
    request: LlmProviderUpdateRequest,
) -> Result<(LlmProfile, bean_core::ApiKeyUpdate), ApiFailure> {
    let LlmProviderUpdateRequest {
        provider,
        model,
        allowed_models,
        api_key_env,
        api_key,
        base_url,
        max_tokens,
    } = request;
    let key_update = match api_key {
        None => bean_core::ApiKeyUpdate::Keep,
        Some(key) if key.trim().is_empty() => bean_core::ApiKeyUpdate::Clear,
        Some(key) => bean_core::ApiKeyUpdate::Set(key.trim().to_string()),
    };
    Ok((
        bean_types::config::LlmProfile {
            name: String::new(), // router gán lại từ tên trên đường dẫn API
            provider: parse_provider_kind(provider.trim())?,
            model: model.trim().to_string(),
            allowed_models: allowed_models
                .into_iter()
                .map(|entry| entry.trim().to_string())
                .filter(|entry| !entry.is_empty())
                .collect(),
            api_key_env: api_key_env.trim().to_string(),
            api_key: None, // key đi qua `key_update`, không qua profile thô
            base_url: base_url
                .map(|url| url.trim().to_string())
                .filter(|url| !url.is_empty()),
            max_tokens: max_tokens.unwrap_or_else(bean_types::config::default_max_tokens),
        },
        key_update,
    ))
}

/// Dựng [`bean_types::config::LlmProfile`] từ request tạo. Tên giữ nguyên bản —
/// `validate_profile_name` chủ động chặn khoảng trắng/ký tự lạ, kể cả khoảng
/// trắng đầu/cuối (người dùng phải tự nhập sạch).
fn parse_create_profile(request: LlmProviderCreateRequest) -> Result<LlmProfile, ApiFailure> {
    let LlmProviderCreateRequest {
        name,
        provider,
        model,
        allowed_models,
        api_key_env,
        api_key,
        base_url,
        max_tokens,
    } = request;
    let (mut profile, key_update) = parse_update_profile(LlmProviderUpdateRequest {
        provider,
        model,
        allowed_models,
        api_key_env,
        api_key,
        base_url,
        max_tokens,
    })?;
    profile.name = name;
    // Tạo profile: key truyền thẳng vào profile (router normalize + validate).
    if let bean_core::ApiKeyUpdate::Set(key) = key_update {
        profile.api_key = Some(key);
    }
    Ok(profile)
}

/// `GET /api/config/llm` — cấu hình LLM runtime, đã khử secret.
pub(super) async fn get_llm_config(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<LlmConfigResponse>> {
    let _ = require_user(&state, &jar).await?;
    llm_config_response(&state).map(Json)
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
    llm_config_response(&state).map(Json)
}

/// `POST /api/config/llm/providers` — thêm profile vào `[[llm.providers]]` của
/// `bean.toml` (persist TRƯỚC khi cập nhật RAM — lỗi đĩa giữ nguyên trạng thái).
pub(super) async fn create_llm_provider_profile(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<LlmProviderCreateRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<LlmConfigResponse>)> {
    let _ = require_user(&state, &jar).await?;
    ensure_profiles_writable(&state)?;
    let profile = parse_create_profile(request)?;
    state
        .router
        .add_llm_provider(profile)
        .map_err(map_profile_error)?;
    let snapshot = llm_config_response(&state)?;
    Ok((axum::http::StatusCode::CREATED, Json(snapshot)))
}

/// `PUT /api/config/llm/providers/{name}` — sửa profile. Tên lấy từ đường dẫn;
/// nếu profile đang active, runtime rebuild provider với trường mới ngay.
pub(super) async fn update_llm_provider_profile(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
    Json(request): Json<LlmProviderUpdateRequest>,
) -> ApiResult<Json<LlmConfigResponse>> {
    let _ = require_user(&state, &jar).await?;
    ensure_profiles_writable(&state)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "profile_required",
            "profile không được rỗng",
        ));
    }
    let (profile, key_update) = parse_update_profile(request)?;
    state
        .router
        .update_llm_provider(name, profile, key_update)
        .map_err(map_profile_error)?;
    llm_config_response(&state).map(Json)
}

/// `DELETE /api/config/llm/providers/{name}` — xoá profile; chặn xoá profile
/// đang active (409 `profile_active`), không tự fallback.
pub(super) async fn delete_llm_provider_profile(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
) -> ApiResult<Json<LlmConfigResponse>> {
    let _ = require_user(&state, &jar).await?;
    ensure_profiles_writable(&state)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiFailure::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "profile_required",
            "profile không được rỗng",
        ));
    }
    state
        .router
        .delete_llm_provider(name)
        .map_err(map_profile_error)?;
    llm_config_response(&state).map(Json)
}
