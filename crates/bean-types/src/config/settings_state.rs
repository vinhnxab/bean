//! Trạng thái cài đặt runtime persist qua restart (M29 mục 7).
//!
//! Gộp `llm-state.json` (profile/model LLM) và `voice-state.json` (bật/tắt voice,
//! provider, ngôn ngữ, voice id) thành một file `data.dir/settings-state.json` để
//! mọi cài đặt không-secret nằm ở một nơi duy nhất. Secret (api key, token, mật khẩu)
//! vẫn ở `secrets.env` — **không** trộm vào JSON này.
//!
//! Nguyên tắc "hành động mới nhất trên UI là ý cuối cùng": file tồn tại ⇒ thắng seed
//! `bean.toml`; vắng mặt ⇒ theo seed. File hỏng ⇒ warn + bỏ qua, không chặn khởi động.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::*;

/// Tên file trạng thái gộp trong `data.dir`.
pub const SETTINGS_STATE_FILE: &str = "settings-state.json";

/// Mọi cài đặt runtime không-secret persist qua restart, gộp thành 1 file JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsState {
    /// Tên profile LLM đang dùng (runtime hot-swap qua UI / `/model`).
    pub llm_profile: String,
    /// Model LLM đang dùng (runtime hot-swap qua UI / `/model`).
    pub llm_model: String,
    /// `true` = mic push-to-talk hiện trong UI; `false` = tắt.
    pub voice_enabled: bool,
    /// Nhà cung cấp STT/TTS người dùng chọn; `None` = giữ seed `[voice].provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_provider: Option<SpeechProviderKind>,
    /// Ngôn ngữ STT/TTS (ISO, vd `vi`); `None` = giữ seed `[voice].language`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_language: Option<String>,
    /// Voice ID TTS mặc định (tiếng Anh + fallback); `None` = giữ seed `bean.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_tts_voice_id: Option<String>,
    /// Voice ID TTS cho văn bản tiếng Việt; `None` = giữ seed `bean.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_tts_voice_vi: Option<String>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            llm_profile: DEFAULT_PROFILE_NAME.to_string(),
            llm_model: String::new(), // được fill từ seed khi áp
            voice_enabled: false,
            voice_provider: None,
            voice_language: None,
            voice_tts_voice_id: None,
            voice_tts_voice_vi: None,
        }
    }
}

impl SettingsState {
    /// Đọc state; file chưa tồn tại ⇒ `None` (không phải lỗi — lần chạy đầu).
    /// File hỏng ⇒ warn + `None`: mất lựa chọn là chấp nhận được, chặn khởi động thì không.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "không đọc được settings-state");
                return None;
            }
        };
        match serde_json::from_str::<Self>(&raw) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "settings-state hỏng — bỏ qua");
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

    /// Áp state vào config sau khi kiểm tra (gộp llm + voice).
    ///
    /// * Profile biến mất khỏi `bean.toml` ⇒ warn, giữ profile seed.
    /// * Model không còn trong `allowed_models` của profile ⇒ fallback về model mặc định.
    /// * `voice_enabled = true` nhưng `[voice]` chưa đủ cấu hình ⇒ giữ seed (không "hồi sinh").
    ///
    /// # Parameters
    /// * `config` — config đã load từ `bean.toml` (sẽ bị mutate).
    #[must_use]
    pub fn apply_to_config(&self, config: &mut Config) -> bool {
        // ── LLM: profile + model ──
        let profile_name = if self.llm_profile.trim().is_empty() {
            DEFAULT_PROFILE_NAME.to_string()
        } else {
            self.llm_profile.trim().to_string()
        };
        let mut applied_llm = true;
        if let Some(profile) = config.llm.profile(&profile_name) {
            let model = if profile
                .effective_allowed_models()
                .iter()
                .any(|allowed| allowed == &self.llm_model)
            {
                self.llm_model.clone()
            } else {
                tracing::warn!(
                    profile = profile_name,
                    model = %self.llm_model,
                    fallback = %profile.model,
                    "model trong settings-state không còn được phép — fallback về model mặc định của profile"
                );
                profile.model.clone()
            };
            config.llm.apply_profile(&LlmProfile {
                model,
                ..profile.clone()
            });
        } else {
            tracing::warn!(
                profile = profile_name,
                "settings-state trỏ tới profile không còn trong bean.toml — dùng cấu hình mặc định"
            );
            applied_llm = false;
        }

        // ── Voice: provider / ngôn ngữ / voice id ──
        if let Some(provider) = self.voice_provider {
            config.voice.provider = provider;
        }
        if let Some(language) = &self.voice_language {
            config.voice.language = language.clone();
        }
        if let Some(voice_id) = &self.voice_tts_voice_id {
            config.voice.tts_voice_id = voice_id.clone();
        }
        if let Some(voice_vi) = &self.voice_tts_voice_vi {
            config.voice.tts_voice_vi = Some(voice_vi.clone());
        }

        // enabled áp sau cùng — chỉ nhận khi `[voice]` đã đủ cấu hình sau khi gộp.
        if self.voice_enabled && !config.voice_configured() {
            tracing::warn!(
                "settings-state bật voice nhưng `[voice]` thiếu cấu hình — bỏ qua cờ enabled, giữ seed từ bean.toml"
            );
            return false;
        }
        config.voice.enabled = self.voice_enabled;

        applied_llm
    }

    /// Migration từ file cũ: nếu `llm-state.json` hoặc `voice-state.json` tồn tại,
    /// đọc và gộp vào `settings-state.json`, rồi xoá file cũ.
    ///
    /// `api_key` từ `voice-state.json` cũ **không** được kéo sang — key đã được migrate
    /// sang `secrets.env` qua `bean config migrate-secrets` trước đó (hoặc vẫn còn env).
    /// Gọi an toàn nhiều lần (idempotent): nếu `settings-state.json` đã tồn tại mà các
    /// file cũ không còn, không làm gì.
    #[must_use]
    pub fn migrate_from_legacy(data_dir: &Path) -> bool {
        let llm_legacy = data_dir.join(LLM_STATE_FILE);
        let voice_legacy = data_dir.join(VOICE_STATE_FILE);
        let target = settings_state_path(data_dir);

        // Nếu target đã tồn tại và cả hai legacy đều vắng mặt ⇒ đã migrate trước đó.
        if target.is_file() && !llm_legacy.is_file() && !voice_legacy.is_file() {
            return true;
        }

        // Xây dựng state từ seed mặc định, rồi đè bằng giá trị từ file cũ (nếu có).
        let mut state = SettingsState::default();

        if let Some(llm) = LlmState::load(&llm_legacy) {
            state.llm_profile = llm.profile;
            state.llm_model = llm.model;
        }
        if let Some(voice) = VoiceState::load(&voice_legacy) {
            state.voice_enabled = voice.enabled;
            state.voice_provider = voice.provider;
            state.voice_language = voice.language;
            state.voice_tts_voice_id = voice.tts_voice_id;
            state.voice_tts_voice_vi = voice.tts_voice_vi;
            // `voice.api_key` khỏi legacy được BỎ QUA — key nằm ở secrets.env.
        }

        // Chỉ ghi file khi có ít nhất một legacy tồn tại (tránh tạo file rỗng không cần).
        if (llm_legacy.is_file() || voice_legacy.is_file())
            && let Err(error) = Self::save(&target, &state)
        {
            tracing::warn!(error = %error, "migrate settings-state thất bại");
            return false;
        }

        // Xoá file legacy sau khi đã ghi thành công target.
        let mut ok = true;
        if llm_legacy.is_file()
            && let Err(error) = std::fs::remove_file(&llm_legacy)
        {
            tracing::warn!(path = %llm_legacy.display(), error = %error, "không xoá được llm-state.json legacy");
            ok = false;
        }
        if voice_legacy.is_file()
            && let Err(error) = std::fs::remove_file(&voice_legacy)
        {
            tracing::warn!(path = %voice_legacy.display(), error = %error, "không xoá được voice-state.json legacy");
            ok = false;
        }
        ok
    }
}

/// Áp state đã persist (nếu có) vào config; `data_dir` phải đã mở rộng `~`.
///
/// Trả `true` khi state tồn tại và hợp lệ. Đường đọc/áp này **không bao giờ lỗi
/// ngoài `false`** — startup không được phép phụ thuộc file state.
#[must_use]
pub fn apply_persisted_settings_state(config: &mut Config, data_dir: &Path) -> bool {
    let Some(state) = SettingsState::load(&settings_state_path(data_dir)) else {
        return false;
    };
    state.apply_to_config(config)
}

/// Đường dẫn file state từ `data.dir` **đã mở rộng** `~`.
#[must_use]
pub fn settings_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SETTINGS_STATE_FILE)
}

/// Đường state file cho config hiện tại; tự mở rộng `~` trong `data.dir`.
///
/// Dùng cho handler `PATCH /api/settings` — bean-web chỉ có `Config` thô.
///
/// # Errors
/// [`ConfigError`] khi `data.dir` dạng `~` mà không xác định được `$HOME`.
pub fn settings_state_path_from_config(config: &Config) -> Result<PathBuf, ConfigError> {
    let data_dir = super::error::expand_tilde(&config.data.dir)?;
    Ok(settings_state_path(&data_dir))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn state() -> SettingsState {
        SettingsState {
            llm_profile: "default".into(),
            llm_model: "m2".into(),
            voice_enabled: true,
            voice_provider: Some(SpeechProviderKind::Api),
            voice_language: Some("vi".into()),
            voice_tts_voice_id: Some("cgSgspJ2msm6clMCkdW9".into()),
            voice_tts_voice_vi: Some("CQAD6iKxS73fEAGjSwt5".into()),
        }
    }

    #[test]
    fn roundtrip_save_load() {
        let dir = temp_dir();
        let path = settings_state_path(dir.path());
        SettingsState::save(&path, &state()).unwrap();
        assert_eq!(SettingsState::load(&path), Some(state()));
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = temp_dir();
        assert_eq!(SettingsState::load(&settings_state_path(dir.path())), None);
    }

    #[test]
    fn load_corrupt_file_is_none() {
        let dir = temp_dir();
        let path = settings_state_path(dir.path());
        std::fs::write(&path, "{không phải json").unwrap();
        assert_eq!(SettingsState::load(&path), None);
    }

    #[test]
    fn save_sets_owner_only_permissions() {
        let dir = temp_dir();
        let path = settings_state_path(dir.path());
        SettingsState::save(&path, &state()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "file chứa cài đặt phải là 0600");
        }
    }

    #[test]
    fn migrate_from_legacy_reads_both_files() {
        let dir = temp_dir();
        let data_dir = dir.path();

        // Ghi legacy files.
        LlmState::save(
            &data_dir.join(LLM_STATE_FILE),
            &LlmState {
                profile: "nvidia".into(),
                model: "z-ai/glm-5.3-flash".into(),
            },
        )
        .unwrap();
        VoiceState::save(
            &data_dir.join(VOICE_STATE_FILE),
            &VoiceState {
                enabled: true,
                provider: Some(SpeechProviderKind::Api),
                language: Some("vi".into()),
                api_key: Some("sk-should-be-migrated-away".into()),
                tts_voice_id: Some("voice-1".into()),
                tts_voice_vi: Some("voice-vi-1".into()),
            },
        )
        .unwrap();

        assert!(SettingsState::migrate_from_legacy(data_dir));

        // Target tồn tại.
        let target = settings_state_path(data_dir);
        assert!(target.is_file());
        let merged = SettingsState::load(&target).unwrap();
        assert_eq!(merged.llm_profile, "nvidia");
        assert_eq!(merged.llm_model, "z-ai/glm-5.3-flash");
        assert_eq!(merged.voice_enabled, true);
        assert_eq!(merged.voice_provider, Some(SpeechProviderKind::Api));
        assert_eq!(merged.voice_language.as_deref(), Some("vi"));
        assert_eq!(merged.voice_tts_voice_id.as_deref(), Some("voice-1"));
        assert_eq!(merged.voice_tts_voice_vi.as_deref(), Some("voice-vi-1"));

        // Legacy files bị xoá.
        assert!(!data_dir.join(LLM_STATE_FILE).is_file());
        assert!(!data_dir.join(VOICE_STATE_FILE).is_file());
    }

    #[test]
    fn migrate_from_legacy_skips_api_key() {
        let dir = temp_dir();
        let data_dir = dir.path();

        VoiceState::save(
            &data_dir.join(VOICE_STATE_FILE),
            &VoiceState {
                enabled: false,
                provider: None,
                language: None,
                api_key: Some("sk-legacy-key".into()),
                tts_voice_id: None,
                tts_voice_vi: None,
            },
        )
        .unwrap();

        assert!(SettingsState::migrate_from_legacy(data_dir));
        let merged = SettingsState::load(&settings_state_path(data_dir)).unwrap();
        // Không có trường api_key trong SettingsState — key đã được migrate sang secrets.env.
        assert!(!merged.voice_enabled);
    }

    #[test]
    fn migrate_from_legacy_idempotent() {
        let dir = temp_dir();
        let data_dir = dir.path();

        // Chỉ có target, không có legacy ⇒ không làm gì.
        SettingsState::save(&settings_state_path(data_dir), &state()).unwrap();
        assert!(SettingsState::migrate_from_legacy(data_dir));
        assert!(settings_state_path(data_dir).is_file());
    }

    #[test]
    fn apply_to_config_applies_llm_and_voice() {
        let mut config = Config::default();
        config.llm.allowed_models = vec!["m2".to_string()];
        config.voice.whisper_model = "/tmp/whisper.bin".into();
        config.voice.piper_model = "/tmp/piper.onnx".into();

        let saved = state();
        assert!(saved.apply_to_config(&mut config));
        assert_eq!(config.llm.model, "m2");
        assert_eq!(config.llm.active_profile_name(), "default");
        assert!(config.voice.enabled);
        assert_eq!(config.voice.provider, SpeechProviderKind::Api);
        assert_eq!(config.voice.language, "vi");
        assert_eq!(config.voice.tts_voice_id, "cgSgspJ2msm6clMCkdW9");
        assert_eq!(
            config.voice.tts_voice_vi.as_deref(),
            Some("CQAD6iKxS73fEAGjSwt5")
        );
    }

    #[test]
    fn apply_to_config_voice_enabled_rejected_when_not_configured() {
        let mut config = Config::default(); // [voice] chưa đủ cấu hình

        let saved = SettingsState {
            llm_profile: "default".into(),
            llm_model: config.llm.model.clone(),
            voice_enabled: true,
            voice_provider: None,
            voice_language: None,
            voice_tts_voice_id: None,
            voice_tts_voice_vi: None,
        };
        // apply_to_config trả false vì voice chưa cấu hình.
        assert!(!saved.apply_to_config(&mut config));
        assert!(!config.voice.enabled);
    }
}
