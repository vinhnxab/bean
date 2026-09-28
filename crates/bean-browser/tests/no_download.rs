//! Chứng minh Bean **không có đường tải Chrome lúc chạy** (M26, nguyên tắc
//! "cài trước, không tải lúc chạy").
//!
//! Đây là bảo đảm ở **tầng build**, không phải lời hứa: crate
//! `chromiumoxide_fetcher` (đối tượng tải Chrome) phải vắng mặt khỏi
//! `Cargo.lock`. Nếu ai đó đổi `default-features = false` thành `true`, test này
//! đỏ — trước khi bản release chứa đường tải về.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Đọc `Cargo.lock` ở gốc workspace, hoặc trả `Err` (không `panic!` — crate này bị
/// cấm `clippy::panic`).
fn workspace_lock() -> Result<String, String> {
    // `CARGO_MANIFEST_DIR` = crates/bean-browser ⇒ lên hai cấp là gốc workspace.
    let dir: PathBuf = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "thiếu CARGO_MANIFEST_DIR".to_string())?
        .into();
    let root = dir
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| "crate nằm trong crates/<name>".to_string())?;
    let path = root.join("Cargo.lock");
    std::fs::read_to_string(&path).map_err(|e| format!("không đọc được {}: {e}", path.display()))
}

/// Không có `chromiumoxide_fetcher` trong lock ⇒ binary không thể tải Chrome.
#[test]
fn chrome_fetcher_is_absent_from_the_dependency_graph() {
    let lock = workspace_lock().expect("đọc được Cargo.lock");
    assert!(
        !lock.contains("chromiumoxide_fetcher"),
        "Cargo.lock chứa `chromiumoxide_fetcher` — Bean sẽ có khả năng TẢI Chrome \
         lúc chạy, vi phạm nguyên tắc \"cài trước, không tải lúc chạy\". \
         Sửa: chromiumoxide phải dùng `default-features = false`"
    );
}

/// Bản ghim phải đúng `=0.9.1`, không phải dải mở.
///
/// CDP type được **sinh tự động** từ `protocol.json`; một bản minor mới có thể đổi
/// shape của type, nên để `chromiumoxide = "0.9"` là khiến build không tái lập
/// được. `=X.Y.Z` là bắt buộc.
#[test]
fn chromiumoxide_version_is_pinned_exactly() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root")
            .join("Cargo.toml"),
    )
    .expect("đọc được Cargo.toml gốc");
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("chromiumoxide ="))
        .expect("workspace khai chromiumoxide");
    assert!(
        line.contains("=0.9.1"),
        "phải ghim đúng phiên bản `=0.9.1`, dòng hiện tại: {line}"
    );
    assert!(
        line.contains("default-features = false"),
        "phải tắt `default-features` để loại `fetcher`, dòng hiện tại: {line}"
    );
}
