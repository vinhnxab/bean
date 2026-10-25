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

/// Bật/tắt voice đang có hiệu lực, persist qua restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceState {
    /// `true` = mic push-to-talk hiện trong UI, STT/TTS nhận request; `false` = ngược lại.
    pub enabled: bool,
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
    /// `enabled = true` chỉ nhận khi `[voice]` còn khai báo đủ (`voice_configured`) —
    /// state của phiên cũ không "hồi sinh" một `[voice]` đã bị xoá/thiếu model; khi đó
    /// trả `false` và giữ nguyên config (warn).
    #[must_use]
    pub fn apply_to_config(&self, config: &mut Config) -> bool {
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

    #[test]
    fn roundtrip_save_load() {
        let dir = temp_dir();
        let path = state_path(dir.path());
        VoiceState::save(&path, &VoiceState { enabled: false }).unwrap();
        assert_eq!(VoiceState::load(&path), Some(VoiceState { enabled: false }));

        VoiceState::save(&path, &VoiceState { enabled: true }).unwrap();
        assert_eq!(VoiceState::load(&path), Some(VoiceState { enabled: true }));
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
        let applied = VoiceState { enabled: true }.apply_to_config(&mut config);
        assert!(!applied);
        assert!(!config.voice.enabled);

        // Tắt thì luôn nhận được — không hồi sinh gì cả.
        let applied = VoiceState { enabled: false }.apply_to_config(&mut config);
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

        let applied = VoiceState { enabled: true }.apply_to_config(&mut config);
        assert!(applied);
        assert!(config.voice.enabled);
    }
}
