//! Trạng thái lựa chọn provider LLM persist qua restart (Giai đoạn 2 hot-swap).
//!
//! `bean.toml` là **tuyên bố** (declared state): profile nào tồn tại, model mặc định.
//! Còn profile/model **đang dùng** là trạng thái runtime do người dùng đổi qua UI
//! hoặc `/model` — nếu chỉ nằm trong RAM thì restart sẽ quay về giá trị file, làm
//! mất lựa chọn của người dùng một cách âm thầm. Vì vậy nó được ghi ra
//! `data.dir/llm-state.json` (tách khỏi `bean.toml` để quá trình ghi 1 dòng JSON
//! không bao giờ đụng tới file cấu hình lớn — ghi hỏng cấu hình là tai hoạ, ghi
//! hỏng state chỉ là fallback về mặc định).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::*;

/// Tên file trạng thái trong `data.dir`.
pub const LLM_STATE_FILE: &str = "llm-state.json";

/// Lựa chọn profile + model đang có hiệu lực, persist qua restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmState {
    /// Tên profile (`"default"` = profile ngầm định từ `[llm]`).
    pub profile: String,
    /// Model đang dùng.
    pub model: String,
}

impl LlmState {
    /// Đọc state; file chưa tồn tại ⇒ `None` (không phải lỗi — lần chạy đầu).
    /// File hỏng ⇒ warn + `None`: mất lựa chọn là chấp nhận được, chặn khởi động thì không.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "không đọc được llm-state");
                return None;
            }
        };
        match serde_json::from_str::<Self>(&raw) {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "llm-state hỏng — bỏ qua");
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

    /// Áp state vào config sau khi validate.
    ///
    /// * Profile biến mất khỏi `bean.toml` ⇒ `false`, config giữ nguyên (warn).
    /// * Model không còn trong `allowed_models` của profile ⇒ vẫn áp profile nhưng
    ///   fallback về `profile.model` (warn) — ít phiền hơn chặn khởi động.
    #[must_use]
    pub fn apply_to_config(&self, config: &mut Config) -> bool {
        let profile_name = if self.profile.trim().is_empty() {
            DEFAULT_PROFILE_NAME
        } else {
            self.profile.trim()
        };
        let Some(profile) = config.llm.profile(profile_name) else {
            tracing::warn!(
                profile = profile_name,
                "llm-state trỏ tới profile không còn trong bean.toml — dùng cấu hình mặc định"
            );
            return false;
        };
        let model = if profile
            .effective_allowed_models()
            .iter()
            .any(|allowed| allowed == &self.model)
        {
            self.model.clone()
        } else {
            tracing::warn!(
                profile = profile_name,
                model = %self.model,
                fallback = %profile.model,
                "model trong llm-state không còn được phép — fallback về model mặc định của profile"
            );
            profile.model.clone()
        };
        config.llm.apply_profile(&LlmProfile { model, ..profile });
        true
    }
}

/// Áp state đã persist (nếu có) vào config; `data_dir` phải đã mở rộng `~`.
///
/// Trả `true` khi state tồn tại và hợp lệ. Đường đọc/áp này **không bao giờ lỗi
/// ngoài `false`** — startup không được phép phụ thuộc file state.
#[must_use]
pub fn apply_persisted_llm_state(config: &mut Config, data_dir: &Path) -> bool {
    let Some(state) = LlmState::load(&state_path(data_dir)) else {
        return false;
    };
    state.apply_to_config(config)
}

/// Đường dẫn file state từ `data.dir` **đã mở rộng** `~`.
#[must_use]
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LLM_STATE_FILE)
}
