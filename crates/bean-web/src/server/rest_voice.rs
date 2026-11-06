//! Handler REST cho voice push-to-talk (M27): trạng thái, cấu hình, STT và TTS.
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
//! * `PATCH /api/voice` persist ở `data.dir/voice-state.json` (mẫu `llm-state.json`):
//!   lựa chọn provider/ngôn ngữ/key từ UI sống sót qua restart, và **không bao giờ** ghi
//!   giá trị key vào audit hay response.
//! * Đổi provider/ngôn ngữ/key dựng lại provider ngay lúc chạy (`set_speech_provider`)
//!   — không cần restart hay sửa `bean.toml`; toggle-only thì tái dùng provider cũ.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bean_speech::{AudioInput, SpeechError, SpeechProvider};
use bean_types::config::{SpeechProviderKind, VoiceConfig, VoiceState, voice_state_path};

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
    match saved_voice_state(state) {
        Some(saved) => saved.enabled,
        None => state.config.voice.enabled,
    }
}

/// Đọc state voice đã persist (nếu có); lỗi đĩa/file hỏng coi như chưa có gì lưu.
fn saved_voice_state(state: &WebState) -> Option<VoiceState> {
    voice_state_path(&state.config)
        .ok()
        .as_deref()
        .and_then(VoiceState::load)
}

/// Bản sao `[voice]` seed **đã áp state file** — lựa chọn từ UI (provider/ngôn ngữ/key)
/// thắng seed `bean.toml`, "hành động mới nhất trên UI là ý cuối cùng".
///
/// Đây là nền duy nhất cho cả `voice_status` lẫn gộp request PATCH: một hàm gộp thì
/// hai handler không bao giờ trả lệch nhau về cấu hình hiệu lực. Cờ `enabled` **không**
/// được gộp ở đây — nó thuộc [`voice_runtime_enabled`] (state file có thể chỉ lưu toggle).
///
/// `pub(super)` cho M29: `rest_settings::test_voice` probe đúng cấu hình hiệu lực này,
/// không phải seed — probe một cấu hình mà UI đang hiển thị thì kết luận mới khớp.
pub(super) fn effective_voice_config(state: &WebState) -> VoiceConfig {
    let mut voice = state.config.voice.clone();
    if let Some(saved) = saved_voice_state(state) {
        if let Some(provider) = saved.provider {
            voice.provider = provider;
        }
        if let Some(language) = &saved.language {
            voice.language = language.clone();
        }
        if let Some(key) = &saved.api_key {
            voice.api_key = Some(key.clone());
        }
        if let Some(voice_id) = &saved.tts_voice_id {
            voice.tts_voice_id = voice_id.clone();
        }
        if let Some(voice_vi) = &saved.tts_voice_vi {
            voice.tts_voice_vi = Some(voice_vi.clone());
        }
    }
    voice
}

/// `true` khi chế độ `api` có thể resolve được key: field đã lưu (từ UI/bean.toml —
/// `effective_voice_config` đã gộp state file vào đây) hoặc biến môi trường.
fn voice_api_key_set(state: &WebState, voice: &VoiceConfig) -> bool {
    let snapshot = state.router.secret_snapshot();
    let get_env = bean_settings::env_first_lookup(snapshot.as_ref());
    // `api_key` còn trong `voice-state.json` cũ (trước M29) vẫn được tính là "đã có"
    // để UI không hiện sai; resolver chạy thật thì bỏ qua trường này.
    voice
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
        || get_env(&voice.api_key_env).is_some()
}

/// `GET /api/voice` — UI dùng để ẩn/hiện nút mic và dựng form cấu hình.
pub(super) async fn voice_status(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<VoiceStatusResponse>> {
    let _ = require_user(&state, &jar).await?;
    let voice = effective_voice_config(&state);
    // `configured`: provider đã dựng được (cấu hình đủ); `enabled`: configured **và**
    // chưa bị tắt qua UI. Tách hai khái niệm để UI hiện form cấu hình (thay vì công
    // tắc hoạt động giả) khi `[voice]` thiếu model/key.
    let configured = state.speech_configured().await;
    Ok(Json(VoiceStatusResponse {
        enabled: configured && voice_runtime_enabled(&state),
        configured,
        provider: voice.provider.as_str().to_string(),
        language: voice.language.clone(),
        max_speak_chars: voice.max_speak_chars,
        api_key_set: voice_api_key_set(&state, &voice),
        tts_voice_id: voice.tts_voice_id.clone(),
        tts_voice_vi: voice.tts_voice_vi.clone().unwrap_or_default(),
    }))
}

/// `PATCH /api/voice` — bật/tắt voice **và** đổi provider/ngôn ngữ/API key runtime.
///
/// Chỉ admin (cùng quy tắc với toggle tool — voice đổi bề mặt thu thập audio). Thứ tự
/// bắt buộc: **kiểm tra cấu hình mới dựng được trước, persist sau, trả response cuối** —
/// request hỏng không để lại state nửa vời trên đĩa, lỗi đĩa không đổi trạng thái runtime
/// (mẫu `rest_config.rs`).
///
/// Toggle-only (không kèm provider/language/key) khi provider đã dựng được thì **không**
/// dựng lại — chỉ ghi state, nhờ vậy tắt/bật nhanh và không tái tạo subprocess/API client.
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
            "chỉ admin được đổi cấu hình voice",
        ));
    }

    // Nền = seed + state file (lựa chọn cũ từ UI), rồi request thắng cả hai.
    let mut next = effective_voice_config(&state);
    let mut config_changed = false;
    if let Some(provider) = request.provider.as_deref() {
        let parsed = SpeechProviderKind::from_config_str(provider).ok_or_else(|| {
            ApiFailure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_provider",
                "provider chỉ nhận `local` hoặc `api`",
            )
        })?;
        config_changed |= parsed != next.provider;
        next.provider = parsed;
    }
    if let Some(language) = request.language.as_deref() {
        let trimmed = language.trim();
        if trimmed.is_empty() {
            return Err(ApiFailure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_language",
                "language rỗng — ví dụ `vi` hoặc `en`",
            ));
        }
        config_changed |= trimmed != next.language;
        next.language = trimmed.to_string();
    }
    // Chuỗi rỗng là ý định XOÁ key đã lưu (fallback về `api_key_env`), không phải lỗi.
    // M29: key ElevenLabs chỉ sống ở `secrets.env` (0600) dưới slot `voice.api_key_env`
    // — không bao giờ ghi vào `voice-state.json`/`bean.toml`. Ý định key chỉ được ghi
    // đĩa SAU khi mọi validation + dựng provider thành công (mặc dưới) — request hỏng
    // không để lại dấu vết trên đĩa.
    let key_changed = request.api_key.is_some();
    let key_intent: Option<Option<String>> = request.api_key.as_deref().map(|key| {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    if key_intent.is_some() {
        // Cũ: dán key vào `next.api_key`. Giờ để None — key nằm trong secrets.env.
        next.api_key = None;
        config_changed = true;
    }
    // Voice id mặc định là bắt buộc (TTS cần voice id trong URL) nên rỗng là lỗi;
    // voice id tiếng Việt thì rỗng là ý định xoá override (rơi về giọng mặc định).
    if let Some(voice_id) = request.tts_voice_id.as_deref() {
        let trimmed = voice_id.trim();
        if trimmed.is_empty() {
            return Err(ApiFailure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_voice_id",
                "voice id rỗng — nhập voice ID ElevenLabs (elevenlabs.io → Voices) hoặc giữ nguyên giá trị hiện tại",
            ));
        }
        config_changed |= trimmed != next.tts_voice_id;
        next.tts_voice_id = trimmed.to_string();
    }
    if let Some(voice_id) = request.tts_voice_vi.as_deref() {
        let trimmed = voice_id.trim().to_string();
        let new_value = (!trimmed.is_empty()).then_some(trimmed);
        config_changed |= new_value != next.tts_voice_vi;
        next.tts_voice_vi = new_value;
    }

    // Dựng provider mới khi cấu hình đổi — hoặc khi chưa có provider nào (lần đầu cấu
    // hình từ UI trên seed chưa đủ). Toggle-only trên provider sống thì bỏ qua.
    let mut rebuilt: Option<Arc<dyn SpeechProvider>> = None;
    if config_changed || !state.speech_configured().await {
        let mut candidate = state.config.clone();
        candidate.voice = next.clone();
        if !candidate.voice_configured() {
            return Err(ApiFailure::new(
                StatusCode::CONFLICT,
                "voice_not_configured",
                "voice chưa cấu hình đủ — chế độ `local` cần whisper/piper model, chế độ `api` cần đủ endpoint/model; bổ sung trong bean.toml hoặc nhập API key ở form trên",
            ));
        }
        let api_key = match &key_intent {
            // Ý định Set: dựng thử với key mới CHƯA ghi đĩa — build hỏng thì key cũ
            // vẫn nguyên.
            Some(Some(key)) => Some(secrecy::SecretString::from(key.clone())),
            // Ý định Clear: dựng thử không key (nếu bắt buộc sẽ fail ở dưới).
            Some(None) => None,
            None => {
                let snapshot = state.router.secret_snapshot();
                let get_env = bean_settings::env_first_lookup(snapshot.as_ref());
                candidate.resolve_voice_api_key_lenient_with(&get_env)
            }
        };
        match bean_speech::build_speech(&candidate, api_key) {
            Ok(Some(provider)) => rebuilt = Some(provider),
            Ok(None) => {
                return Err(ApiFailure::new(
                    StatusCode::CONFLICT,
                    "voice_not_configured",
                    "voice chưa cấu hình đủ — UI sẽ hiển thị form cấu hình",
                ));
            }
            Err(error) => {
                // Chi tiết lỗi (đường dẫn binary/model thiếu) chỉ vào log server;
                // client là người dùng đã đăng nhập nhưng không cần thiết phải thấy.
                tracing::warn!(error = %error, "PATCH /api/voice: dựng provider thất bại");
                return Err(ApiFailure::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "voice_not_configured",
                    "dựng provider voice thất bại — kiểm tra key/binary/model rồi thử lại",
                ));
            }
        }
    }

    // Ghi ý định key vào `secrets.env` — chỉ tới được khi mọi bước trên thành công.
    if let Some(intent) = &key_intent {
        let store = state.router.secret_store().ok_or_else(|| {
            ApiFailure::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "secret_unavailable",
                "tiến trình không có khoá secret — cấu hình key qua biến môi trường",
            )
        })?;
        let env_name = next.api_key_env.clone();
        let registry = bean_settings::SecretSlotRegistry::from_names(&[env_name.as_str()]);
        let result = match intent {
            Some(key) => store.set(
                &registry,
                &env_name,
                secrecy::SecretString::from(key.clone()),
            ),
            None => store.remove(&registry, &env_name).map(|_existed| ()),
        };
        if result.is_err() {
            return Err(ApiFailure::internal());
        }
    }

    // Persist cả lựa chọn cấu hình (provider/language/key) lẫn toggle vào một file —
    // restart sau đó `apply_persisted_voice_state` dựng lại đúng provider đã chọn.
    let saved = VoiceState {
        enabled: request.enabled,
        provider: Some(next.provider),
        language: Some(next.language.clone()),
        api_key: next.api_key.clone(),
        tts_voice_id: Some(next.tts_voice_id.clone()),
        tts_voice_vi: next.tts_voice_vi.clone(),
    };
    let path = voice_state_path(&state.config).map_err(|_| ApiFailure::internal())?;
    VoiceState::save(&path, &saved).map_err(|_| {
        ApiFailure::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "voice_state_unwritable",
            "không ghi được voice-state.json — kiểm tra quyền ghi data.dir",
        )
    })?;
    if let Some(provider) = rebuilt {
        state.set_speech_provider(provider).await;
    }
    record_update_audit(&state, &user_id, &next, request.enabled, key_changed);
    Ok(Json(VoiceStatusResponse {
        enabled: request.enabled,
        configured: true,
        provider: next.provider.as_str().to_string(),
        language: next.language.clone(),
        max_speak_chars: state.config.voice.max_speak_chars,
        api_key_set: voice_api_key_set(&state, &next),
        tts_voice_id: next.tts_voice_id.clone(),
        tts_voice_vi: next.tts_voice_vi.clone().unwrap_or_default(),
    }))
}

/// Ghi audit cho mỗi lần đổi voice (mục 15.8): ai quyết định, bật/tắt, provider nào,
/// có đổi key không. **Không bao giờ** ghi giá trị key — chỉ cờ boolean.
fn record_update_audit(
    state: &WebState,
    user_id: &str,
    voice: &VoiceConfig,
    enabled: bool,
    key_changed: bool,
) {
    let Some(log) = state.audit.as_ref() else {
        return;
    };
    let mut entry = bean_security::entry_now(
        0,
        "web",
        "voice.update",
        &serde_json::json!({
            "enabled": enabled,
            "provider": voice.provider.as_str(),
            "key_changed": key_changed,
        }),
    );
    entry.ok = Some(true);
    entry.decision = "allow";
    entry.decided_by = user_id.to_string();
    if let Err(error) = log.record(&entry) {
        tracing::warn!(error = %error, "không ghi được audit voice update");
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
    let provider = speech_provider(&state).await?;
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
    let provider = speech_provider(&state).await?;
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
async fn speech_provider(state: &WebState) -> Result<Arc<dyn SpeechProvider>, ApiFailure> {
    state.speech_provider().await.ok_or_else(speech_disabled)
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
