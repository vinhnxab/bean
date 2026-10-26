//! Trạng thái bật/tắt voice persist qua restart (toggle trong UI, M27).
//!
//! Giống [`super::llm_state`]: `bean.toml` là **tuyên bố** — `[voice].enabled` chỉ là
//! giá trị seed lúc khởi động. Còn việc bật/tắt mic trong màn Cài đặt là trạng thái
//! runtime: nếu chỉ nằm trong RAM thì restart âm thầm quay về giá trị file, làm mất
//! lựa chọn của người dùng. Vì vậy nó được ghi ra `data.dir/voice-state.json` (tách
//! khỏi `bean.toml` — ghi hỏng state chỉ là fallback về seed, không phải tai hoạ).
//!
//! Ý nghĩa "hành động mới nhất trên UI là ý cuối cùng" y hệt `tool_overrides`: file
//! tồn tại ⇒ thắng seed; vắng mặt ⇒ theo `[voice].enabled`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::*;

/// Tên file trạng thái trong `data.dir`.
pub const VOICE_STATE_FILE: &str = "voice-state.json";

/// Trạng thái runtime của voice, persist qua restart.
///
/// Ngoài `enabled` (M27), lưu thêm lựa chọn nhà cung cấp/ngôn ngữ/API key mà người
/// dùng đặt từ UI (`PATCH /api/voice`) — mọi trường đều optional: `None` = giữ seed
/// từ `bean.toml`. File chứa API key nên `save` luôn đặt quyền `0600`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceState {
    /// `true` = mic push-to-talk hiện trong UI, STT/TTS nhận request; `false` = ngược lại.
    pub enabled: bool,
    /// Nhà cung cấp STT/TTS người dùng chọn; `None` = giữ seed `[voice].provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<SpeechProviderKind>,
    /// Ngôn ngữ STT/TTS (ISO, vd `vi`); `None` = giữ seed `[voice].language`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// API key chế độ `api` (ưu tiên hơn `api_key_env`); `None` = resolve từ env.
    /// **Không bao giờ in qua `Debug`** — impl thủ công bên dưới đã che.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

// Derive `Debug` sẽ in key nguyên vẹn khi ai đó log state — impl thủ công chỉ trả
// độ dài dạng `Some(48 ký tự)` để debug vẫn biết key đã có hay chưa.
impl std::fmt::Debug for VoiceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceState")
            .field("enabled", &self.enabled)
            .field("provider", &self.provider)
            .field("language", &self.language)
            .field(
                "api_key",
                &self
                    .api_key
                    .as_ref()
                    .map(|key| format!("{} ký tự", key.chars().count())),
            )
            .finish()
    }
}

impl VoiceState {
    /// Đọc state; file chưa tồn tại ⇒ `None` (không phải lỗi — lần chạy đầu).
    /// File hỏng ⇒ warn + `None`: mất toggle là chấp nhận được, chặn khởi động thì không.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "không đọc được voice-state");
                return None;
            }
        };
        match serde_json::from_str::<Self>(&raw) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "voice-state hỏng — bỏ qua");
                None
            }
        }
    }

    /// Ghi state atomically (tmp + rename) với quyền `0600` trên unix.
    ///
    /// # Errors
    /// [`std::io::Error`] khi không tạo được thư mục/ghi file.
    pub fn save(path: &Path, state: &Self) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string(state)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        // Rename trong cùng filesystem là atomic — không có lúc nào file ở trạng thái nửa vời.
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Áp state vào config sau khi kiểm tra.
    ///
    /// Lựa chọn provider/ngôn ngữ/key từ UI được áp **trước** khi xét `enabled`:
    /// `enabled = true` chỉ nhận khi `[voice]` sau khi gộp còn khai báo đủ
    /// (`voice_configured`) — state của phiên cũ không "hồi sinh" một `[voice]`
    /// đã bị xoá/thiếu model; khi đó trả `false` và giữ nguyên cờ `enabled` seed.
    #[must_use]
    pub fn apply_to_config(&self, config: &mut Config) -> bool {
        if let Some(provider) = self.provider {
            config.voice.provider = provider;
        }
        if let Some(language) = &self.language {
            config.voice.language = language.clone();
        }
        if let Some(api_key) = &self.api_key {
            config.voice.api_key = Some(api_key.clone());
        }
        if self.enabled && !config.voice_configured() {
            tracing::warn!(
                "voice-state bật nhưng `[voice]` thiếu cấu hình — bỏ qua, dùng seed từ bean.toml"
            );
            return false;
        }
        config.voice.enabled = self.enabled;
        true
    }
}

/// Áp state đã persist (nếu có) vào config; `data_dir` phải đã mở rộng `~`.
///
/// Trả `true` khi state tồn tại và hợp lệ. Đường đọc/áp này **không bao giờ lỗi
/// ngoài `false`** — startup không được phép phụ thuộc file state.
#[must_use]
pub fn apply_persisted_voice_state(config: &mut Config, data_dir: &Path) -> bool {
    let Some(state) = VoiceState::load(&state_path(data_dir)) else {
        return false;
    };
    state.apply_to_config(config)
}

/// Đường dẫn file state từ `data.dir` **đã mở rộng** `~`.
#[must_use]
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(VOICE_STATE_FILE)
}

/// Đường state file cho config hiện tại; tự mở rộng `~` trong `data.dir`.
///
/// Dùng cho handler `PATCH /api/voice` — bean-web chỉ có `Config` thô.
///
/// # Errors
/// [`ConfigError`] khi `data.dir` dạng `~` mà không xác định được `$HOME`.
pub fn voice_state_path(config: &Config) -> Result<PathBuf, ConfigError> {
    let data_dir = super::error::expand_tilde(&config.data.dir)?;
    Ok(state_path(&data_dir))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// State tối giản (chỉ `enabled`) — hình dạng file cũ trước khi mở rộng.
    fn state(enabled: bool) -> VoiceState {
        VoiceState {
            enabled,
            provider: None,
            language: None,
            api_key: None,
        }
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = temp_dir();
        let path = state_path(dir.path());
        VoiceState::save(&path, &state(false)).unwrap();
        assert_eq!(VoiceState::load(&path), Some(state(false)));

        VoiceState::save(&path, &state(true)).unwrap();
        assert_eq!(VoiceState::load(&path), Some(state(true)));
    }

    #[test]
    fn roundtrip_keeps_provider_language_and_key() {
        let dir = temp_dir();
        let path = state_path(dir.path());
        let saved = VoiceState {
            enabled: true,
            provider: Some(SpeechProviderKind::Api),
            language: Some("vi".into()),
            api_key: Some("sk-secret-key-abcd".into()),
        };
        VoiceState::save(&path, &saved).unwrap();
        assert_eq!(VoiceState::load(&path).as_ref(), Some(&saved));
    }

    #[test]
    fn legacy_file_with_only_enabled_still_loads() {
        let dir = temp_dir();
        let path = state_path(dir.path());
        std::fs::write(&path, r#"{"enabled":true}"#).unwrap();
        assert_eq!(VoiceState::load(&path), Some(state(true)));
    }

    #[test]
    fn save_sets_owner_only_permissions() {
        let dir = temp_dir();
        let path = state_path(dir.path());
        VoiceState::save(
            &path,
            &VoiceState {
                enabled: false,
                provider: Some(SpeechProviderKind::Api),
                language: None,
                api_key: Some("sk-x".into()),
            },
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "file chứa key phải là 0600");
        }
    }

    #[test]
    fn debug_never_shows_api_key() {
        let with_key = VoiceState {
            enabled: true,
            provider: Some(SpeechProviderKind::Api),
            language: None,
            api_key: Some("sk-never-leak-please".into()),
        };
        let rendered = format!("{with_key:?}");
        assert!(!rendered.contains("sk-never-leak-please"), "{rendered}");
        // Độ dài kỳ vọng tính từ chính key để không lệch khi sửa chuỗi sample.
        let expected = format!("{} ký tự", "sk-never-leak-please".chars().count());
        assert!(rendered.contains(&expected), "{rendered}");
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = temp_dir();
        assert_eq!(VoiceState::load(&state_path(dir.path())), None);
    }

    #[test]
    fn load_corrupt_file_is_none() {
        let dir = temp_dir();
        let path = state_path(dir.path());
        std::fs::write(&path, "{không phải json").unwrap();
        assert_eq!(VoiceState::load(&path), None);
    }

    #[test]
    fn apply_ignores_enable_when_voice_not_configured() {
        // Config mặc định: `[voice]` không có model → chưa cấu hình đủ.
        let mut config = Config::default();
        assert!(!config.voice_configured());
        let applied = state(true).apply_to_config(&mut config);
        assert!(!applied);
        assert!(!config.voice.enabled);

        // Tắt thì luôn nhận được — không hồi sinh gì cả.
        let applied = state(false).apply_to_config(&mut config);
        assert!(applied);
        assert!(!config.voice.enabled);
    }

    #[test]
    fn apply_enable_when_voice_configured() {
        let mut config = Config::default();
        config.voice.enabled = false;
        config.voice.whisper_model = "/tmp/whisper.bin".into();
        config.voice.piper_model = "/tmp/piper.onnx".into();
        assert!(config.voice_configured());

        let applied = state(true).apply_to_config(&mut config);
        assert!(applied);
        assert!(config.voice.enabled);
    }

    #[test]
    fn apply_overrides_provider_language_and_key() {
        let mut config = Config::default();
        let saved = VoiceState {
            enabled: false,
            provider: Some(SpeechProviderKind::Api),
            language: Some("en".into()),
            api_key: Some("sk-from-ui".into()),
        };
        assert!(saved.apply_to_config(&mut config));
        assert_eq!(config.voice.provider, SpeechProviderKind::Api);
        assert_eq!(config.voice.language, "en");
        assert_eq!(config.voice.api_key.as_deref(), Some("sk-from-ui"));
        // `enabled = false` áp bình thường.
        assert!(!config.voice.enabled);
    }

    #[test]
    fn apply_stored_key_makes_api_configured() {
        // Key nằm trong state file (đặt từ UI) phải đủ để `voice_configured` — người
        // dùng không cần đặt biến môi trường khi đã nhập key từ UI.
        let mut config = Config::default();
        assert!(!config.voice_configured());
        let saved = VoiceState {
            enabled: false,
            provider: Some(SpeechProviderKind::Api),
            language: None,
            api_key: Some("sk-from-ui".into()),
        };
        assert!(saved.apply_to_config(&mut config));
        assert!(config.voice_configured());
    }
}
