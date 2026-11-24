//! Marker "đã cài bản mới, chờ restart" (M31).
//!
//! `bean update` cài binary mới xuống đĩa nhưng tiến trình `bean serve` đang chạy
//! vẫn là bản cũ. Khi restart bị bỏ qua (`--no-restart`), không tìm thấy
//! `beanctl`, hoặc restart thất bại, updater ghi một marker nhỏ vào `data.dir`
//! để banner UI (đọc qua `GET /api/status`) nhắc người dùng khởi động lại.
//!
//! Vòng đời:
//!
//! 1. updater cài binary thành công và **không chắc** đã có tiến trình mới chạy
//!    → ghi marker.
//! 2. `bean serve` khởi động thành công (đang chạy bản vừa cài) → xoá marker
//!    ngay lúc boot; nhờ vậy banner tự biến mất sau restart thật.
//!
//! Marker chỉ là tiện ích UX — mất file không ảnh hưởng tính đúng đắn: restart
//! tay vẫn áp dụng bản mới như thường.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Tên file marker trong `data.dir`.
pub const UPDATE_PENDING_FILE: &str = "update-pending.json";

/// Nội dung marker: tag vừa cài + thời điểm cài (giây Unix, mang tính thông tin).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePending {
    /// Tag release vừa được cài xuống đĩa (ví dụ `v0.3.0`).
    pub tag: String,
    /// Thời điểm cài, giây Unix.
    pub installed_at: u64,
}

/// Đường dẫn marker trong `data_dir`.
pub fn pending_marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(UPDATE_PENDING_FILE)
}

/// Ghi marker "chờ restart" vào `data_dir` (tạo thư mục nếu thiếu, quyền `0600`).
pub fn write_pending_marker(data_dir: &Path, tag: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let installed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let pending = UpdatePending {
        tag: tag.to_string(),
        installed_at,
    };
    let json = serde_json::to_string(&pending)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(pending_marker_path(data_dir))?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::create(pending_marker_path(data_dir))?;
    file.write_all(json.as_bytes())
}

/// Đọc marker nếu có; file hỏng coi như không có (marker chỉ là UX, không chặn).
pub fn read_pending_marker(data_dir: &Path) -> Option<UpdatePending> {
    let text = std::fs::read_to_string(pending_marker_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Xoá marker (best-effort); `bean serve` gọi lúc boot thành công.
pub fn clear_pending_marker(data_dir: &Path) {
    match std::fs::remove_file(pending_marker_path(data_dir)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!(
            error = %error,
            dir = %data_dir.display(),
            "không xoá được marker update-pending"
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn write_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        write_pending_marker(dir.path(), "v1.2.3").unwrap();
        let pending = read_pending_marker(dir.path()).expect("marker phải đọc được");
        assert_eq!(pending.tag, "v1.2.3");
        assert!(pending.installed_at > 0);
    }

    #[test]
    fn read_missing_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_pending_marker(dir.path()), None);
    }

    #[test]
    fn read_corrupt_is_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(UPDATE_PENDING_FILE), "không phải json").unwrap();
        assert_eq!(read_pending_marker(dir.path()), None);
    }

    #[test]
    fn write_creates_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data");
        write_pending_marker(&nested, "v2.0.0").unwrap();
        assert!(read_pending_marker(&nested).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn marker_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write_pending_marker(dir.path(), "v1.0.0").unwrap();
        let mode = std::fs::metadata(pending_marker_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn clear_removes_and_tolerates_missing() {
        let dir = tempfile::tempdir().unwrap();
        clear_pending_marker(dir.path()); // chưa có — không được panic
        write_pending_marker(dir.path(), "v1.0.0").unwrap();
        clear_pending_marker(dir.path());
        assert_eq!(read_pending_marker(dir.path()), None);
    }
}
