//! Nạp file và [`Config::validate`] — điểm vào duy nhất khi đọc `bean.toml`.

use std::path::Path;

use super::*;

impl Config {
    /// Đọc `bean.toml` từ `path`, parse và kiểm tra.
    ///
    /// # Errors
    /// * [`ConfigError::Read`] khi không đọc được file.
    /// * [`ConfigError::Parse`] khi TOML sai cú pháp/sai kiểu/khoá lạ.
    /// * [`ConfigError::Invalid`] khi cấu hình vô nghĩa hoặc không an toàn.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut config: Self = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }
}
impl Config {
    /// Nạp cấu hình từ `path` nếu có; nếu không thì dùng giá trị mặc định (có cảnh báo).
    ///
    /// Đây là đường đi của `bean chat` khi người dùng chưa tạo `bean.toml`
    /// (`docs/decisions.md` D5.11).
    ///
    /// # Errors
    /// Như [`Config::load`].
    pub fn load_or_default(path: Option<&Path>) -> Result<Self, ConfigError> {
        match path {
            Some(path) => Self::load(path),
            None => {
                tracing::warn!(
                    "chưa có bean.toml — dùng giá trị mặc định; xem bean.example.toml để cấu hình đầy đủ"
                );
                let mut config = Self::default();
                config.validate()?;
                Ok(config)
            }
        }
    }
}
impl Config {
    /// Kiểm tra ngữ nghĩa, chuẩn hoá `~` và `web.public_origin`.
    ///
    /// Những gì **không** kiểm ở đây: sự tồn tại của `data.dir`/`workspace` (tạo ở M3/M5),
    /// tính hợp lệ của múi giờ IANA (cần `chrono-tz`, cài ở M13), và sự tồn tại của API key
    /// (kiểm khi gọi [`Config::resolve_secrets_with`]).
    ///
    /// # Errors
    /// Trả [`ConfigError::Invalid`] khi cấu hình vô nghĩa hoặc không an toàn.
    pub fn validate(&mut self) -> Result<(), ConfigError> {
        self.validate_core()?;
        self.validate_rbac()?;
        self.validate_billing()?;
        self.validate_marketing()?;
        self.validate_infra_scope()?;
        self.validate_qa()?;
        self.validate_projects()?;
        self.validate_web_and_channels()?;
        self.validate_browser()?;
        self.validate_mcp_servers()?;
        self.validate_mcp_servers()?;
        self.validate_mcp_server()?;
        self.validate_paths()
    }
}
