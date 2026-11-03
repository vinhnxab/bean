//! Speech-to-text và text-to-speech cho push-to-talk web (agents.md mục 12, M27).
//!
//! Hai chế độ nhà cung cấp (mục 9.1):
//! * [`LocalSpeech`] — tiến trình con `whisper-cli` (STT) + `piper` (TTS) trên máy chủ.
//!   Không mạng, không audio rời khỏi máy; âm thanh được ghi vào file tạm rồi xoá.
//! * [`ElevenLabsSpeech`] — ElevenLabs qua HTTPS. Audio **rời khỏi máy** tới nhà cung cấp;
//!   cần API key riêng.
//!
//! [`FakeSpeech`] dùng cho test và `--fake-speech`: trả lại transcript/synthétic predetermined
//! mà không cần mạng hay binary thật.
//!
//! # Bất biến
//! * `transcribe`/`synthesize` **không bao giờ panic**; mọi lỗi trả [`SpeechError`].
//! * API key bọc `secrecy::SecretString`, không bao giờ in ra log/`Debug`.
//! * Mỗi lời gọi có timeout (lấy từ cấu hình) — binary treo hay API chậm không giữ request.

#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use bean_types::config::{Config, SpeechProviderKind};
use secrecy::SecretString;
use thiserror::Error;

mod api;
mod local;

pub use api::ElevenLabsSpeech;
pub use local::LocalSpeech;

/// Chunk audio nhận từ trình duyệt (webm/opus, wav...) hoặc file tạm.
#[derive(Debug, Clone)]
pub struct AudioInput {
    /// Byte audio thô.
    pub data: Vec<u8>,
    /// MIME type, ví dụ `audio/webm` (dùng để đặt phần mở rộng file tạm cho `whisper-cli`).
    pub mime: String,
}

/// Audio đã tổng hợp, sẵn sàng gửi về trình duyệt phát.
#[derive(Debug, Clone)]
pub struct AudioOutput {
    /// Byte audio thô (ví dụ MPEG/MP3 từ ElevenLabs, WAV từ Piper).
    pub data: Vec<u8>,
    /// MIME type để trình duyệt biết cách phát, ví dụ `audio/mpeg`.
    pub mime: String,
}

/// Lỗi STT/TTS.
#[derive(Debug, Error)]
pub enum SpeechError {
    /// Voice chưa được cấu hình đủ (thiếu model/binary/key) — nên chặn ngay lúc khởi động.
    #[error("voice chưa cấu hình: {0}")]
    NotConfigured(String),

    /// Binary hoặc file model local không tồn tại/không chạy được.
    #[error("không chạy được tiến trình voice: {0}")]
    NotFound(String),

    /// Vượt quá `voice.timeout_seconds`.
    #[error("quá thời gian chờ voice ({0:?})")]
    Timeout(Duration),

    /// Lỗi khác (HTTP, subprocess trả về mã lỗi, response sai định dạng).
    #[error("lỗi voice: {0}")]
    Failed(String),
}

/// Nhà cung cấp STT/TTS — trait mỏng, tương tự provider LLM.
///
/// Cả hai hàm nhận timeout đã lưu trong đối tượng và bọc lời gọi bằng
/// `tokio::time::timeout` nội bộ, nên caller REST không phải lo về thời gian.
#[async_trait::async_trait]
pub trait SpeechProvider: Send + Sync {
    /// Chuyển audio thành văn bản (STT).
    ///
    /// # Errors
    /// [`SpeechError`] khi subprocess/API lỗi hoặc quá thời gian.
    async fn transcribe(&self, audio: &AudioInput) -> Result<String, SpeechError>;

    /// Chuyển văn bản thành audio (TTS).
    ///
    /// # Errors
    /// [`SpeechError`] khi subprocess/API lỗi hoặc quá thời gian.
    async fn synthesize(&self, text: &str) -> Result<AudioOutput, SpeechError>;
}

/// Dựng provider theo cấu hình; trả `None` khi `[voice]` chưa khai báo đủ.
///
/// Tách "đã cấu hình" khỏi "đang bật": provider dựng khi cấu hình đầy đủ **kể cả khi
/// `enabled = false`** — nhờ vậy toggle trong UI (`PATCH /api/voice`) bật lại được
/// không cần restart. Cờ `enabled` chỉ còn là trạng thái runtime, được kiểm tra tại
/// từng request trong `bean-web`.
///
/// # Errors
/// [`SpeechError::NotConfigured`] khi chế độ `api` thiếu API key. `serve` dịch điều này
/// thành "chưa cấu hình" (warn + `None`) khi seed tắt; chỉ fail khởi động tường minh
/// (D6.4) khi seed `[voice].enabled = true`.
pub fn build_speech(
    config: &Config,
    api_key: Option<SecretString>,
) -> Result<Option<Arc<dyn SpeechProvider>>, SpeechError> {
    let voice = &config.voice;
    if !config.voice_configured() {
        return Ok(None);
    }
    let timeout = Duration::from_secs(voice.timeout_seconds.max(1));
    let provider: Arc<dyn SpeechProvider> = match voice.provider {
        SpeechProviderKind::Local => Arc::new(LocalSpeech::new(
            voice.whisper_command.clone(),
            voice.whisper_model.clone(),
            voice.piper_command.clone(),
            voice.piper_model.clone(),
            voice.language.clone(),
            timeout,
        )),
        SpeechProviderKind::Api => {
            let key = api_key.ok_or_else(|| {
                SpeechError::NotConfigured(
                    "chế độ `api` yêu cầu API key — đặt biến `voice.api_key_env`".into(),
                )
            })?;
            Arc::new(ElevenLabsSpeech::from_config(voice, key)?)
        }
    };
    Ok(Some(provider))
}

/// Cắt chuỗi tại **ranh giới ký tự UTF-8**, không bao giờ tách giữa hai byte (mục 22.9).
#[must_use]
pub fn truncate_utf8(text: &str, max: usize) -> &str {
    if text.chars().count() <= max {
        return text;
    }
    match text.char_indices().nth(max) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

/// `true` khi văn bản chứa ký tự riêng của bảng chữ tiếng Việt.
///
/// Heuristic chọn giọng TTS giữa tiếng Việt và ngôn ngữ Latin không dấu (tiếng Anh):
/// * Khối Unicode `U+1EA0..=U+1EFF` (Latin Extended Additional) chứa gần hết các
///   nguyên âm có dấu tổ hợp của tiếng Việt (`ạ ế ở ữ ỷ…`);
/// * Phần còn lại là các ký tự Latin-1/Latin Extended-A tiếng Việt dùng riêng
///   (`ă â đ ê ô ơ ư`) và các dấu huyền/sắc/ngã trên nguyên âm trần (`à á ã è é…`),
///   mà tiếng Anh **không bao giờ** chứa.
///
/// Chấp nhận nhận nhầm văn bản Pháp/Tây Ban Nha trích trong câu trả lời — hiếm và
/// vô hại (chỉ khác giọng đọc); không có dấu thì rơi về giọng mặc định.
#[must_use]
pub fn has_vietnamese_diacritics(text: &str) -> bool {
    text.chars().any(|c| {
        ('\u{1EA0}'..='\u{1EFF}').contains(&c)
            || matches!(
                c,
                'ă' | 'Ă'
                    | 'â'
                    | 'Â'
                    | 'đ'
                    | 'Đ'
                    | 'ê'
                    | 'Ê'
                    | 'ô'
                    | 'Ô'
                    | 'ơ'
                    | 'Ơ'
                    | 'ư'
                    | 'Ư'
                    | 'à'
                    | 'À'
                    | 'á'
                    | 'Á'
                    | 'ã'
                    | 'Ã'
                    | 'è'
                    | 'È'
                    | 'é'
                    | 'É'
                    | 'ì'
                    | 'Ì'
                    | 'í'
                    | 'Í'
                    | 'ò'
                    | 'Ò'
                    | 'ó'
                    | 'Ó'
                    | 'ù'
                    | 'Ù'
                    | 'ú'
                    | 'Ú'
                    | 'ý'
                    | 'Ý'
                    | 'ũ'
                    | 'Ũ'
            )
    })
}

/// Provider STT/TTS giả cho test và `--fake-speech` — không mạng, không subprocess.
pub struct FakeSpeech {
    /// Transcript giả trả về khi transcribe.
    pub transcript: String,
}

impl FakeSpeech {
    /// Tạo fake với transcript cho trước.
    #[must_use]
    pub fn new(transcript: impl Into<String>) -> Self {
        Self {
            transcript: transcript.into(),
        }
    }
}

#[async_trait::async_trait]
impl SpeechProvider for FakeSpeech {
    async fn transcribe(&self, _audio: &AudioInput) -> Result<String, SpeechError> {
        Ok(self.transcript.clone())
    }

    async fn synthesize(&self, text: &str) -> Result<AudioOutput, SpeechError> {
        Ok(AudioOutput {
            data: text.as_bytes().to_vec(),
            mime: "audio/mpeg".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{has_vietnamese_diacritics, truncate_utf8};

    #[test]
    fn truncate_utf8_is_char_boundary_safe() {
        assert_eq!(truncate_utf8("xin chào", 100), "xin chào");
        assert_eq!(truncate_utf8("Xin chào các bạn", 4), "Xin ");
        let emoji = "🧸🧸🧸 teddy";
        let cut = truncate_utf8(emoji, 3);
        assert_eq!(cut, "🧸🧸🧸");
        assert!(cut.is_char_boundary(cut.len()));
    }

    #[test]
    fn vietnamese_diacritics_detected() {
        assert!(has_vietnamese_diacritics("Xin chào"));
        assert!(has_vietnamese_diacritics("là sao vậy"));
        assert!(has_vietnamese_diacritics("Tôi đang ở nhà"));
        assert!(has_vietnamese_diacritics("đường phố"));
        assert!(has_vietnamese_diacritics("hỏi thời tiết Hà Nội"));
        // Nguyên âm có dấu tổ hợp nằm trong khối U+1EA0..=U+1EFF.
        assert!(has_vietnamese_diacritics("trường học"));
        assert!(has_vietnamese_diacritics("thỉnh thoảng"));
    }

    #[test]
    fn non_vietnamese_text_has_no_diacritics() {
        assert!(!has_vietnamese_diacritics(""));
        assert!(!has_vietnamese_diacritics("Hello world!"));
        assert!(!has_vietnamese_diacritics("What's the weather like today?"));
        assert!(!has_vietnamese_diacritics("123 + 456 = 579"));
    }
}
