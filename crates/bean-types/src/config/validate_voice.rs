//! Kiểm tra cấu hình `[voice]` (M27).
//!
//! Chặn ở **tầng config** những tổ hợp vô nghĩa hoặc không an toàn, để không phải "phát
//! hiện khi gọi API" (nguyên tắc đã dùng cho `forbid_tags` M21.6 và `validate_billing`):

use super::error::invalid;
use super::validate::validate_http_url;
use super::*;

impl Config {
    /// Kiểm tra `[voice]` (M27).
    ///
    /// Khi voice bật:
    /// * `language`, `timeout_seconds`, `max_speak_chars` phải hợp lệ;
    /// * chế độ `local` phải trỏ tới **cả** binary lẫn model (whisper + piper) — thiếu thì
    ///   lỗi khởi động tường minh, không âm thầm fallback (D6.4);
    /// * chế độ `api` phải có `api_key_env` **riêng** (không trùng credential của LLM, web
    ///   search, Telegram, billing, marketing — D13.2) và `api_base_url` phải là http(s).
    pub(super) fn validate_voice(&self) -> Result<(), ConfigError> {
        if !self.voice.enabled {
            return Ok(());
        }
        self.validate_voice_fields(&self.voice)
    }

    /// Kiểm tra các trường `[voice]` **bỏ qua** cờ `enabled` — cùng bộ điều kiện với
    /// [`Config::validate_voice`] nhưng dùng được cả khi seed đang tắt, để
    /// [`Config::voice_configured`] phân biệt "đã cấp phát" với "đang bật".
    pub(super) fn validate_voice_fields(&self, voice: &VoiceConfig) -> Result<(), ConfigError> {
        if voice.language.trim().is_empty() {
            return Err(invalid("[voice].language rỗng — ví dụ `vi` hoặc `en`"));
        }
        if voice.timeout_seconds == 0 {
            return Err(invalid("[voice].timeout_seconds phải > 0"));
        }
        if voice.max_speak_chars == 0 {
            return Err(invalid("[voice].max_speak_chars phải > 0"));
        }
        // Trần cứng: câu thoại quá dài vừa tốn tiền vừa khó nghe; 2000 là ngưỡng phổ biến
        // cho "một câu nói". Cắt ở tầng provider vẫn là lưới an toàn, nhưng cấu hình vô
        // nghĩa (ví dụ 1e9) nên bị chặn ngay.
        if voice.max_speak_chars > 2000 {
            return Err(invalid(
                "[voice].max_speak_chars tối đa 2000 — voice trả lời bằng MỘT câu ngắn (mục 9.5)",
            ));
        }

        match voice.provider {
            SpeechProviderKind::Local => {
                if voice.whisper_command.trim().is_empty() {
                    return Err(invalid(
                        "[voice].whisper_command rỗng — cần tên/đường dẫn `whisper-cli` (chế độ local)",
                    ));
                }
                if voice.whisper_model.trim().is_empty() {
                    return Err(invalid(
                        "[voice].whisper_model rỗng — cần đường dẫn model `.bin` của whisper.cpp",
                    ));
                }
                if voice.piper_command.trim().is_empty() {
                    return Err(invalid(
                        "[voice].piper_command rỗng — cần tên/đường dẫn `piper` (chế độ local)",
                    ));
                }
                if voice.piper_model.trim().is_empty() {
                    return Err(invalid(
                        "[voice].piper_model rỗng — cần đường dẫn model `.onnx` của Piper",
                    ));
                }
            }
            SpeechProviderKind::Api => {
                if voice.api_key_env.trim().is_empty() {
                    return Err(invalid(
                        "[voice].api_key_env rỗng — cần TÊN biến chứa API key",
                    ));
                }
                validate_http_url("[voice].api_base_url", &voice.api_base_url)?;
                if voice.tts_voice_id.trim().is_empty() {
                    return Err(invalid(
                        "[voice].tts_voice_id rỗng — cần voice ID trong voice vault ElevenLabs",
                    ));
                }
                if voice.stt_model.trim().is_empty() {
                    return Err(invalid("[voice].stt_model rỗng"));
                }
                if voice.tts_model.trim().is_empty() {
                    return Err(invalid("[voice].tts_model rỗng"));
                }
                // Chế độ `api` gửi audio sang bên thứ ba — credential phải là biến RIÊNG
                // (D13.2), không dùng lại key có quyền rộng hơn.
                let others: [(&str, &str); 5] = [
                    ("llm.api_key_env", &self.llm.api_key_env),
                    (
                        "tools.web_search.api_key_env",
                        &self.tools.web_search.api_key_env,
                    ),
                    ("telegram.token_env", &self.telegram.token_env),
                    ("billing.api_key_env", &self.billing.api_key_env),
                    ("marketing.api_key_env", &self.marketing.api_key_env),
                ];
                for (field, env) in others {
                    if env.trim() == voice.api_key_env.trim() {
                        return Err(invalid(format!(
                            "[voice].api_key_env không được trùng với {field} (`{env}`): \
                             credential gửi audio phải RIÊNG, tách khỏi các quyền còn lại"
                        )));
                    }
                }
                tracing::warn!(
                    provider = voice.provider.as_str(),
                    "voice chế độ `api`: audio người dùng sẽ được gửi tới nhà cung cấp bên thứ ba"
                );
            }
        }
        Ok(())
    }
}

impl Config {
    /// `[voice]` đã khai báo đủ để dựng provider chưa — **bỏ qua** cờ `enabled`.
    ///
    /// Tách hai khái niệm vốn bị dính vào nhau trước đây:
    /// * *cấp phát* (provisioning): binary/model/key đã khai báo đủ trong `bean.toml`;
    /// * *bật* (enabled): trạng thái runtime, seed từ `[voice].enabled` và bị ghi đè bởi
    ///   toggle trong UI (`data.dir/voice-state.json`, xem module [`super::voice_state`]).
    ///
    /// Provider được dựng lúc khởi động khi mục này `true`, kể cả khi `enabled = false` —
    /// nhờ vậy bật lại từ UI không cần restart.
    #[must_use]
    pub fn voice_configured(&self) -> bool {
        self.validate_voice_fields(&self.voice).is_ok()
    }

    /// Thư mục UI trên đĩa (`[web].ui_dir`, đã mở rộng `~` sau validate) — `None`
    /// khi không đặt, binary dùng bản UI nhúng (feature `ui`, mục 11.3).
    pub fn web_ui_dir(&self) -> Option<&std::path::Path> {
        self.web.ui_dir.as_deref()
    }
}
