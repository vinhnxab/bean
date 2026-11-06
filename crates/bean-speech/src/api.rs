//! Chế độ `api`: ElevenLabs qua HTTPS (M27 mục 9.1).
//!
//! * STT `POST /v1/speech-to-text` (multipart: `file`, `model_id`, `language`) — model
//!   `scribe_v2` hỗ trợ 90+ ngôn ngữ gồm tiếng Việt.
//! * TTS `POST /v1/text-to-speech/{voice_id}` (JSON: `text`, `model_id`) — model
//!   `eleven_multilingual_v2` hỗ trợ tiếng Việt.
//!
//! # Quyền riêng tư
//!
//! Audio **rời khỏi máy** tới ElevenLabs. `validate()` và runtime đều cảnh báo; người dùng
//! phải tự quyết định có bật chế độ này không. API key bọc `secrecy`, chỉ đọc lúc gắn header.

use std::time::Duration;

use bean_types::config::VoiceConfig;
use reqwest::multipart;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::{AudioInput, AudioOutput, SpeechError, SpeechProvider};

/// Response STT của ElevenLabs: chỉ trường `text`.
#[derive(Deserialize)]
struct SttResponse {
    text: String,
}

/// Nhà cung cấp ElevenLabs (STT `scribe_v2` + TTS `eleven_multilingual_v2`).
pub struct ElevenLabsSpeech {
    client: reqwest::Client,
    api_key: SecretString,
    base_url: String,
    stt_model: String,
    tts_model: String,
    /// Giọng TTS mặc định/fallback (văn bản không phải tiếng Việt).
    voice_id: String,
    /// Giọng TTS cho văn bản tiếng Việt (tuỳ chọn) — cấu hình qua `tts_voice_vi` trong
    /// `[voice]` (bean.toml).
    vi_voice: Option<String>,
    /// Giọng TTS fallback (giọng nữ miễn phí) khi giọng chính bị ElevenLabs từ chối (402/403).
    fallback_voice_id: Option<String>,
    language: String,
    timeout: Duration,
}

impl ElevenLabsSpeech {
    /// Dựng provider ElevenLabs từ cấu hình `[voice]`.
    ///
    /// # Errors
    /// [`SpeechError::Failed`] khi không dựng được HTTP client (lỗi khởi tạo TLS — hiếm).
    pub fn from_config(voice: &VoiceConfig, api_key: SecretString) -> Result<Self, SpeechError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("bean/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| SpeechError::Failed(format!("không dựng được HTTP client: {e}")))?;
        Ok(Self {
            client,
            api_key,
            base_url: voice.api_base_url.trim_end_matches('/').to_string(),
            stt_model: voice.stt_model.clone(),
            tts_model: voice.tts_model.clone(),
            voice_id: voice.tts_voice_id.clone(),
            vi_voice: voice.tts_voice_vi.clone(),
            fallback_voice_id: voice
                .fallback_voice_id
                .clone()
                .or_else(|| Some("EXAVITQu4vr4xnSDxMaL".to_string())),
            language: voice.language.clone(),
            timeout: Duration::from_secs(voice.timeout_seconds.max(1)),
        })
    }

    /// Gọi ElevenLabs với timeout đã khai báo; trả [`SpeechError`] đã lọc (không lộ key).
    async fn send(
        &self,
        request: impl std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    ) -> Result<reqwest::Response, SpeechError> {
        let response = tokio::time::timeout(self.timeout, request)
            .await
            .map_err(|_| SpeechError::Timeout(self.timeout))?
            .map_err(|e| SpeechError::Failed(format!("lỗi mạng ElevenLabs: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            // Không in body đầy đủ (có thể chứa thông tin định danh); chỉ in mã lỗi.
            return Err(SpeechError::Failed(format!("ElevenLabs trả HTTP {status}")));
        }
        Ok(response)
    }
}

#[async_trait::async_trait]
impl SpeechProvider for ElevenLabsSpeech {
    async fn transcribe(&self, audio: &AudioInput) -> Result<String, SpeechError> {
        let filename = format!("audio.{}", extension_for(&audio.mime));
        let part = multipart::Part::bytes(audio.data.clone())
            .file_name(filename)
            .mime_str(&audio.mime)
            .map_err(|e| SpeechError::Failed(format!("MIME audio không hợp lệ: {e}")))?;
        let form = multipart::Form::new()
            .part("file", part)
            .text("model_id", self.stt_model.clone())
            .text("language", self.language.clone());
        let url = format!("{}/v1/speech-to-text", self.base_url);
        let request = self
            .client
            .post(&url)
            .header("xi-api-key", self.api_key.expose_secret())
            .multipart(form)
            .send();
        let response = self.send(request).await?;
        let body = response
            .json::<SttResponse>()
            .await
            .map_err(|e| SpeechError::Failed(format!("response STT sai định dạng: {e}")))?;
        Ok(body.text.trim().to_string())
    }

    async fn synthesize(&self, text: &str) -> Result<AudioOutput, SpeechError> {
        // Chọn giọng theo ngôn ngữ của văn bản cần đọc: có dấu tiếng Việt → giọng `vi`
        // (nếu đã cấu hình), còn lại giọng mặc định. Cả hai đều là voice id trong URL.
        let primary_voice_id = if crate::has_vietnamese_diacritics(text) {
            self.vi_voice.as_deref().unwrap_or(&self.voice_id)
        } else {
            self.voice_id.as_str()
        };

        // Thử giọng chính trước. Nếu ElevenLabs từ chối (402 paid_plan_required / 403)
        // — ví dụ giọng Voice Library yêu cầu gói trả phí — rơi về giọng fallback
        // (mặc định Bella, giọng nữ miễn phí) để câu trả lời vẫn được đọc.
        let response = self.post_tts(primary_voice_id, text).await?;
        let status = response.status();
        if status.is_success() {
            return Self::finish_response(response).await;
        }
        if matches!(status.as_u16(), 402 | 403)
            && let Some(fallback_id) = &self.fallback_voice_id
        {
            tracing::warn!(
                voice_id = %primary_voice_id,
                fallback_voice_id = %fallback_id,
                status = status.as_u16(),
                "giọng TTS bị ElevenLabs từ chối — rơi về giọng fallback"
            );
            let fallback = self.post_tts(fallback_id, text).await?;
            let status = fallback.status();
            if status.is_success() {
                return Self::finish_response(fallback).await;
            }
            return Err(SpeechError::Failed(format!(
                "giọng fallback cũng bị ElevenLabs từ chối (HTTP {status})"
            )));
        }
        Err(SpeechError::Failed(format!("ElevenLabs trả HTTP {status}")))
    }
}

impl ElevenLabsSpeech {
    /// POST /v1/text-to-speech/{voice_id} với timeout đã khai báo. Trả response kể cả
    /// khi status lỗi — caller quyết định fallback (voice library bị chặn 402) hay
    /// biến thành [`SpeechError`]. Chỉ dùng cho TTS; `send()` vẫn xử STT.
    async fn post_tts(&self, voice_id: &str, text: &str) -> Result<reqwest::Response, SpeechError> {
        let url = format!("{}/v1/text-to-speech/{}", self.base_url, voice_id);
        let body = serde_json::json!({
            "text": text,
            "model_id": self.tts_model,
        });
        let request = self
            .client
            .post(&url)
            .header("xi-api-key", self.api_key.expose_secret())
            .json(&body)
            .send();
        tokio::time::timeout(self.timeout, request)
            .await
            .map_err(|_| SpeechError::Timeout(self.timeout))?
            .map_err(|e| SpeechError::Failed(format!("lỗi mạng ElevenLabs: {e}")))
    }

    /// Đọc body audio từ response thành [`AudioOutput`].
    async fn finish_response(response: reqwest::Response) -> Result<AudioOutput, SpeechError> {
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("audio/mpeg")
            .to_string();
        let data = response
            .bytes()
            .await
            .map_err(|e| SpeechError::Failed(format!("không đọc được audio TTS: {e}")))?
            .to_vec();
        Ok(AudioOutput { data, mime })
    }
}

/// Phần mở rộng file tạm theo MIME (giống `LocalSpeech`, tách ra dùng chung).
fn extension_for(mime: &str) -> &'static str {
    let mime = mime.to_ascii_lowercase();
    if mime.contains("wav") {
        "wav"
    } else if mime.contains("mpeg") || mime.contains("mp3") {
        "mp3"
    } else if mime.contains("ogg") {
        "ogg"
    } else if mime.contains("mp4") || mime.contains("m4a") {
        "m4a"
    } else {
        "webm"
    }
}
