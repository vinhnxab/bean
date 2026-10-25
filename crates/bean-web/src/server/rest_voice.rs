//! Handler REST cho voice push-to-talk (M27): trạng thái, STT và TTS.
//!
//! # Vì sao là endpoint riêng, không nhét vào WebSocket
//!
//! STT/TTS là request/response **không trạng thái** và luôn gắn với một lần bấm/nghe
//! cụ thể, khác hẳn luồng sự kiện một chiều của run. Nếu đẩy audio qua WS thì mỗi
//! khung hình phải mang thêm nhánh phân loại, và một clip hỏng sẽ chặn cả socket.
//! REST cũng cho phép dùng lại nguyên `request_guard` (Origin/Host/CSRF) mà không
//! phải lặp lại logic bảo mật trong `ws.rs`.
//!
//! # Bất biến
//!
//! * Mọi handler đều `require_user` trước khi chạm tới provider — chưa đăng nhập thì
//!   không suy đoán được voice có bật hay không (403/401 trước, không rò cấu hình).
//! * Audio luôn đi qua JSON + base64 (xem [`VoiceTranscribeRequest`]) để giữ bất biến
//!   `Content-Type: application/json` của `request_guard`.
//! * Voice tắt (chưa cấu hình hoặc bị tắt qua UI) trả `503 voice_disabled` — fail-closed,
//!   không âm thầm gọi provider nào.
//! * Bật/tắt qua `PATCH /api/voice` là trạng thái runtime persist ở
//!   `data.dir/voice-state.json` (mẫu `llm-state.json`): file tồn tại thắng seed
//!   `[voice].enabled`, nhờ vậy toggle trong UI sống sót qua restart.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bean_speech::{AudioInput, SpeechError};
use bean_types::config::{VoiceState, voice_state_path};

use crate::api_types::{
    VoiceStatusResponse, VoiceSynthesizeRequest, VoiceSynthesizeResponse, VoiceTranscribeRequest,
    VoiceTranscribeResponse, VoiceUpdateRequest,
};

use super::{ApiFailure, ApiResult, WebState, require_user};

/// Trạng thái bật/tắt voice **hiệu lực lúc chạy**: file toggle từ UI thắng seed.
///
/// `serve` đã áp file vào `config.voice.enabled` lúc khởi động, nên không có file tức
/// là seed (kèm override đã áp nếu có). Đọc file mỗi request: endpoint voice dùng
/// thưa theo hành động của người dùng, còn một nguồn sự thật duy nhất trên đĩa thì
/// không bao giờ lệch giữa nhiều handler.
fn voice_runtime_enabled(state: &WebState) -> bool {
    match voice_state_path(&state.config)
        .ok()
        .as_deref()
        .map(VoiceState::load)
    {
        Some(Some(saved)) => saved.enabled,
        _ => state.config.voice.enabled,
    }
}

/// `GET /api/voice` — UI dùng để ẩn/hiện nút mic mà không cần thử gọi.
pub(super) async fn voice_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<VoiceStatusResponse>> {
    let _ = require_user(&state, &jar).await?;
    let voice = &state.config.voice;
    // `configured`: provider đã dựng được (cấu hình đủ); `enabled`: configured **và**
    // chưa bị tắt qua UI. Tách hai khái niệm để UI disable toggle (thay vì hiện công
    // tắc hoạt động giả) khi `[voice]` thiếu model/key.
    let configured = state.speech.is_some();
    Ok(Json(VoiceStatusResponse {
        enabled: configured && voice_runtime_enabled(&state),
        configured,
        provider: voice.provider.as_str().to_string(),
        language: voice.language.clone(),
        max_speak_chars: voice.max_speak_chars,
    }))
}

/// `PATCH /api/voice` — bật/tắt voice runtime (màn Cài đặt).
///
/// Chỉ admin (cùng quy tắc với toggle tool — toggle đổi bề mặt thu thập audio).
/// Thứ tự bắt buộc: **persist file trước, trả response sau** — lỗi đĩa không đổi
/// trạng thái (mẫu `rest_config.rs`). Chỉ nhận khi provider đã dựng được: bật từ UI
/// khi thiếu model/key chỉ sinh ra nút mic hỏng, nên bị chặn rõ ràng.
pub(super) async fn voice_update(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<VoiceUpdateRequest>,
) -> ApiResult<Json<VoiceStatusResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let permissions = state.config.permissions_for(&user_id);
    if state.config.rbac_enabled() && permissions.role != "admin" {
        return Err(ApiFailure::new(
            StatusCode::FORBIDDEN,
            "admin_required",
            "chỉ admin được bật/tắt voice",
        ));
    }
    if state.speech.is_none() {
        return Err(ApiFailure::new(
            StatusCode::CONFLICT,
            "voice_not_configured",
            "voice chưa cấu hình đủ trong bean.toml — cần khai báo model/key rồi khởi động lại",
        ));
    }
    let path = voice_state_path(&state.config).map_err(|_| ApiFailure::internal())?;
    VoiceState::save(
        &path,
        &VoiceState {
            enabled: request.enabled,
        },
    )
    .map_err(|_| {
        ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "voice_state_unwritable",
            "không ghi được voice-state.json — kiểm tra quyền ghi data.dir",
        )
    })?;
    record_toggle_audit(&state, &user_id, request.enabled);
    let voice = &state.config.voice;
    Ok(Json(VoiceStatusResponse {
        enabled: request.enabled,
        configured: true,
        provider: voice.provider.as_str().to_string(),
        language: voice.language.clone(),
        max_speak_chars: voice.max_speak_chars,
    }))
}

/// Ghi audit cho mỗi lần toggle voice (mục 15.8): ai quyết định, bật hay tắt.
fn record_toggle_audit(state: &WebState, user_id: &str, enabled: bool) {
    let Some(log) = state.audit.as_ref() else {
        return;
    };
    let mut entry = bean_security::entry_now(
        0,
        "web",
        "voice.toggle",
        &serde_json::json!({ "enabled": enabled }),
    );
    entry.ok = Some(true);
    entry.decision = "allow";
    entry.decided_by = user_id.to_string();
    if let Err(error) = log.record(&entry) {
        tracing::warn!(error = %error, "không ghi được audit toggle voice");
    }
}

/// `POST /api/voice/transcribe` — audio base64 → văn bản.
pub(super) async fn voice_transcribe(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<VoiceTranscribeRequest>,
) -> ApiResult<Json<VoiceTranscribeResponse>> {
    let _ = require_user(&state, &jar).await?;
    if !voice_runtime_enabled(&state) {
        return Err(speech_disabled());
    }
    let provider = speech_provider(&state)?;
    let data = BASE64
        .decode(request.audio_base64.as_bytes())
        .map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_audio",
                "audio_base64 không giải mã được",
            )
        })?;
    if data.is_empty() {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_audio",
            "audio rỗng",
        ));
    }
    let audio = AudioInput {
        data,
        mime: request.mime,
    };
    let text = provider
        .transcribe(&audio)
        .await
        .map_err(|error| speech_failure(&error))?;
    Ok(Json(VoiceTranscribeResponse { text }))
}

/// `POST /api/voice/synthesize` — văn bản (đã cắt theo cấu hình) → audio base64.
pub(super) async fn voice_synthesize(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<VoiceSynthesizeRequest>,
) -> ApiResult<Json<VoiceSynthesizeResponse>> {
    let _ = require_user(&state, &jar).await?;
    if !voice_runtime_enabled(&state) {
        return Err(speech_disabled());
    }
    let provider = speech_provider(&state)?;
    let max = usize::try_from(state.config.voice.max_speak_chars).unwrap_or(usize::MAX);
    let clipped = bean_speech::truncate_utf8(request.text.trim(), max);
    if clipped.is_empty() {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "empty_text",
            "text rỗng",
        ));
    }
    let audio = provider
        .synthesize(clipped)
        .await
        .map_err(|error| speech_failure(&error))?;
    Ok(Json(VoiceSynthesizeResponse {
        audio_base64: BASE64.encode(&audio.data),
        mime: audio.mime,
    }))
}

/// Lấy provider đã dựng; voice tắt/chưa cấu hình là lỗi tường minh, không fallback.
fn speech_provider(
    state: &WebState,
) -> Result<std::sync::Arc<dyn bean_speech::SpeechProvider>, ApiFailure> {
    state.speech.clone().ok_or_else(speech_disabled)
}

/// `503 voice_disabled` — cùng một mã cho "tắt qua UI" và "chưa cấu hình": client
/// chỉ cần biết "không dùng được lúc này", chi tiết là việc của log phía server.
fn speech_disabled() -> ApiFailure {
    ApiFailure::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "voice_disabled",
        "voice chưa được bật",
    )
}

/// Ánh xạ [`SpeechError`] sang HTTP, không rò chi tiết nội bộ ra ngoài.
fn speech_failure(error: &SpeechError) -> ApiFailure {
    match error {
        SpeechError::NotConfigured(_) => ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_not_configured",
            "voice chưa được cấu hình đầy đủ",
        ),
        SpeechError::Timeout(_) => ApiFailure::new(
            StatusCode::GATEWAY_TIMEOUT,
            "voice_timeout",
            "voice quá thời gian chờ",
        ),
        SpeechError::NotFound(_) | SpeechError::Failed(_) => ApiFailure::new(
            StatusCode::BAD_GATEWAY,
            "voice_failed",
            "voice xử lý thất bại",
        ),
    }
}
