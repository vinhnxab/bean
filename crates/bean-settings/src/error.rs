//! Lỗi của kho secret và dịch vụ thiết lập.
//!
//! Mọi biến thể **không bao giờ** chứa giá trị secret — chỉ tên slot, số dòng, đường
//! dẫn hoặc lý do. Thông điệp viết bằng tiếng Việt vì đi thẳng tới người dùng CLI/UI.

use std::path::PathBuf;

/// Lỗi thao tác với `secrets.env` và bảng thiết lập (M29).
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Lỗi I/O khi đọc/ghi file (kèm đường dẫn, không kèm nội dung file).
    #[error("không thao tác được `{path}`: {source}")]
    Io {
        /// File bị đụng tới.
        path: PathBuf,
        /// Lỗi hệ thống gốc.
        #[source]
        source: std::io::Error,
    },
    /// File không đúng định dạng `NAME=value` (sai tên, thiếu `=`, khoá lặp…).
    #[error("`{path}` dòng {line}: {reason}")]
    Parse {
        /// File bị parse.
        path: PathBuf,
        /// Số dòng (1-based) gây lỗi.
        line: usize,
        /// Lý do, không chứa giá trị.
        reason: String,
    },
    /// Tên không thuộc slot nào đã khai báo trong cấu hình hiện tại.
    #[error("`{name}` không phải secret slot đã khai báo — xem `bean secret list`")]
    UnknownSlot {
        /// Tên bị từ chối.
        name: String,
    },
    /// Giá trị vi phạm ràng buộc của `secrets.env` (rỗng, quá dài, ký tự điều khiển).
    #[error("giá trị của `{name}` không hợp lệ: {reason}")]
    InvalidValue {
        /// Slot liên quan.
        name: String,
        /// Lý do, không chứa giá trị.
        reason: String,
    },
    /// Khoá không có trong bảng [`crate::settings::SETTINGS`] — `config set` từ chối
    /// ghi tuỳ ý vào TOML.
    #[error("`{key}` không phải khoá thiết lập — xem `bean config list`")]
    UnknownSetting {
        /// Khoá bị từ chối.
        key: String,
    },
    /// Giá trị không khớp kiểu của khoá (`config set`).
    #[error("`{key}`: {reason}")]
    InvalidSetting {
        /// Khoá liên quan.
        key: String,
        /// Lý do, không chứa giá trị.
        reason: String,
    },
    /// Cấu hình sau khi sửa không qua được `Config::validate` — file **không** được ghi.
    #[error("cấu hình mới bị từ chối, bean.toml giữ nguyên: {message}")]
    ConfigRejected {
        /// Lý do validate từ chối.
        message: String,
    },
    /// File TOML không đọc/ghi/parse được (kèm đường dẫn, không kèm nội dung).
    #[error("`{path}`: {reason}")]
    Toml {
        /// File bị đụng tới.
        path: PathBuf,
        /// Lý do, không chứa nội dung file.
        reason: String,
    },
    /// Không ghi được dòng audit — set/unset bị huỷ (audit là bắt buộc, M29 mục 4.3).
    #[error("không ghi được audit log: {message}")]
    Audit {
        /// Lý do, không chứa giá trị.
        message: String,
    },
}
