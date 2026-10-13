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
//! * Voice tắt (`state.speech = None`) trả `503 voice_disabled` — fail-closed, không
//!   âm thầm gọi provider nào.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bean_speech::{AudioInput, SpeechError};

use crate::api_types::{
    VoiceStatusResponse, VoiceSynthesizeRequest, VoiceSynthesizeResponse, VoiceTranscribeRequest,
    VoiceTranscribeResponse,
};

use super::{ApiFailure, ApiResult, WebState, require_user};

/// `GET /api/voice` — UI dùng để ẩn/hiện nút mic mà không cần thử gọi.
pub(super) async fn voice_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<VoiceStatusResponse>> {
    let _ = require_user(&state, &jar).await?;
    let voice = &state.config.voice;
    Ok(Json(VoiceStatusResponse {
        // `speech.is_some()` mới là sự thật lúc chạy (provider đã dựng được); config
        // có thể bật mà provider chưa sẵn sàng nếu thiếu model/key, và khi đó
        // `serve` đã báo lỗi lúc khởi động. Dùng `speech` để status không nói dối.
        enabled: state.speech.is_some(),
        provider: voice.provider.as_str().to_string(),
        language: voice.language.clone(),
        max_speak_chars: voice.max_speak_chars,
    }))
}

/// `POST /api/voice/transcribe` — audio base64 → văn bản.
pub(super) async fn voice_transcribe(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<VoiceTranscribeRequest>,
) -> ApiResult<Json<VoiceTranscribeResponse>> {
    let _ = require_user(&state, &jar).await?;
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
    state.speech.clone().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_disabled",
            "voice chưa được bật",
        )
    })
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
