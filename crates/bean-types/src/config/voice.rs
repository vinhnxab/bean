//! Section `[voice]` — push-to-talk trên web, trả lời bằng một câu nói ngắn (M27).
//!
//! Hai chế độ nhà cung cấp (mục 9.1 của kế hoạch):
//! * `local` — chạy tiến trình con `whisper-cli` (STT) và `piper` (TTS) trên máy chủ.
//!   Không cần mạng, không dữ liệu rời khỏi máy; yêu cầu người vận hành cài sẵn binary
//!   và model tiếng Việt. Thiếu binary/model → lỗi khởi động tường minh (D6.4), **không**
//!   âm thầm fallback.
//! * `api` — gọi ElevenLabs qua HTTPS (`scribe_v2` để STT, `eleven_multilingual_v2` để
//!   TTS). Cần API key riêng; audio **có rời khỏi máy** — validate() và runtime đều cảnh báo.

use serde::{Deserialize, Serialize};

use super::*;

/// `[voice]` — cấu hình push-to-talk (M27).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceConfig {
    /// Bật tính năng voice. Mặc định `false`.
    pub enabled: bool,
    /// Nhà cung cấp STT/TTS (xem [`SpeechProviderKind`]).
    pub provider: SpeechProviderKind,
    /// Mã ngôn ngữ ISO, ví dụ `vi` (tiếng Việt), `en`.
    pub language: String,
    /// Trần số ký tự gửi đi TTS. Câu trả lời dài hơn bị cắt tại ranh giới ký tự UTF-8
    /// trước khi đọc — tránh chi phí và câu thoại quá dài (mục 9.5).
    pub max_speak_chars: u32,
    /// Thời gian tối đa cho mỗi lần STT/TTS (giây).
    pub timeout_seconds: u64,

    // ---- Chế độ `local` (tiến trình con) ----
    /// Đường dẫn hoặc tên `whisper-cli` (whisper.cpp) để transcribe.
    pub whisper_command: String,
    /// Đường dẫn file model `.bin` của whisper.cpp.
    pub whisper_model: String,
    /// Đường dẫn hoặc tên `piper` để synthesize.
    pub piper_command: String,
    /// Đường dẫn file model `.onnx` của Piper (ví dụ `vi_VN-vivos-x_low.onnx`).
    pub piper_model: String,

    // ---- Chế độ `api` (ElevenLabs) ----
    /// Tên biến môi trường chứa API key ElevenLabs **riêng**.
    ///
    /// `validate()` từ chối nếu trùng credential của LLM/web search/Telegram/billing/
    /// marketing — mẫu D13.2: credential gửi dữ liệu nhạy cảm (audio) phải là key riêng.
    pub api_key_env: String,
    /// Endpoint ElevenLabs. Chỉ đổi khi bạn tự host proxy; mặc định là API chính thức.
    pub api_base_url: String,
    /// Model STT của ElevenLabs (`scribe_v2` hỗ trợ 90+ ngôn ngữ gồm tiếng Việt).
    pub stt_model: String,
    /// Model TTS của ElevenLabs (`eleven_multilingual_v2` hỗ trợ tiếng Việt).
    pub tts_model: String,
    /// Voice ID dùng cho TTS (thuộc voice vault của ElevenLabs, nằm trong URL của API).
    pub tts_voice_id: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: SpeechProviderKind::Local,
            // Tiếng Việt là mặc định theo ngôn ngữ mặc định của UI (agents.md mục 12.1).
            language: "vi".to_string(),
            // 400 ký tự ~ 1 câu nói ngắn; hợp lý với VOICE_GUIDANCE yêu cầu ≤ 40 ký tự/câu.
            max_speak_chars: 400,
            timeout_seconds: 30,
            // Tên lệnh phổ biến; người dùng có thể đổi thành đường dẫn tuyệt đối.
            whisper_command: "whisper-cli".to_string(),
            whisper_model: String::new(),
            piper_command: "piper".to_string(),
            piper_model: String::new(),
            api_key_env: "ELEVENLABS_API_KEY".to_string(),
            api_base_url: "https://api.elevenlabs.io".to_string(),
            stt_model: "scribe_v2".to_string(),
            tts_model: "eleven_multilingual_v2".to_string(),
            // Voice "Rachel" (mặc định phổ biến của ElevenLabs) — đổi trong bean.toml nếu
            // bạn dùng voice khác trong voice vault của mình.
            tts_voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
        }
    }
}
