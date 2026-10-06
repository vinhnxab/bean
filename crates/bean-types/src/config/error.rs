//! Lỗi cấu hình [`ConfigError`] và hàm tiện ích (`invalid`, `expand_tilde`).

use std::path::{Path, PathBuf};

/// Lỗi khi nạp/kiểm tra cấu hình.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Không đọc được file.
    #[error("không đọc được file cấu hình `{path}`: {source}")]
    Read {
        /// Đường dẫn đã thử đọc.
        path: PathBuf,
        /// Lỗi I/O gốc.
        #[source]
        source: std::io::Error,
    },
    /// TOML sai cú pháp hoặc sai kiểu.
    #[error("file cấu hình `{path}` không hợp lệ: {source}")]
    Parse {
        /// Đường dẫn file.
        path: PathBuf,
        /// Lỗi TOML gốc (có số dòng/cột).
        #[source]
        source: toml::de::Error,
    },
    /// Cấu hình đọc được nhưng vô nghĩa/không an toàn.
    #[error("cấu hình sai: {0}")]
    Invalid(String),
    /// Thiếu biến môi trường cho secret.
    #[error("thiếu biến môi trường `{env}` (được khai báo bởi trường `{field}`)")]
    MissingEnv {
        /// Trường trong `bean.toml` khai báo biến này.
        field: &'static str,
        /// Tên biến môi trường.
        env: String,
    },
}

/// Tạo lỗi cấu hình sai.
pub(super) fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}

/// Mở rộng `~`/`~/...` thành `$HOME` (agents.md mục 18: `data.dir = "~/.bean"`).
pub(super) fn expand_tilde(path: &Path) -> Result<PathBuf, ConfigError> {
    let raw = path.to_string_lossy();
    if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| invalid("không xác định được $HOME để mở rộng `~` trong đường dẫn"))?;
        let rest = raw.strip_prefix('~').unwrap_or_default();
        return Ok(PathBuf::from(home).join(rest.trim_start_matches('/')));
    }
    Ok(path.to_path_buf())
}
