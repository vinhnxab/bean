//! Phuc vu UI (nhung hoac thu muc dia) + SPA fallback (muc 11.3).
//!
//! # Bat bien
//!
//! File co hash trong ten (`assets/*`) cache lau; `index.html` `no-cache` (muc 11.3).
//! Va nhu `routes.rs`: `/api/*` khong ton tai phai tra 404 JSON chu khong phai HTML.

use std::path::PathBuf;

use axum::http::{HeaderValue, Uri, header};
use axum::response::{IntoResponse, Response};
use tower_http::set_header::SetResponseHeaderLayer;

use bean_types::Config;

use super::ApiFailure;
pub(super) async fn api_not_found() -> Response {
    ApiFailure::not_found().into_response()
}

/// Nguồn file tĩnh của giao diện (mục 11.3).
///
/// `[web].ui_dir` là nguồn sự thật: đặt thì đọc từ đĩa (nâng cấp UI không phải
/// build lại Rust), không đặt thì dùng bản nhúng — không tự fallback về bản nhúng
/// khi thư mục sai để lỗi cấu hình lộ rõ thay vì lặng lẽ phục vụ UI cũ.
pub(super) enum StaticRoot {
    /// Bản UI nhúng lúc build (feature `ui`) — mặc định khi release.
    ///
    /// Chỉ được construct khi feature `ui` bật; bản headless ghim variant này
    /// để enum đủ ngữ nghĩa cho nhánh match.
    #[cfg_attr(not(feature = "ui"), allow(dead_code))]
    Embedded,
    /// Thư mục UI trên đĩa (`[web].ui_dir`), đã mở rộng `~` lúc validate.
    Dir(PathBuf),
    /// Không có UI sẵn (build headless và không đặt `[web].ui_dir`).
    ///
    /// Chỉ được construct khi feature `ui` tắt; bản có UI ghim variant này
    /// để enum đủ ngữ nghĩa cho nhánh match.
    #[cfg_attr(feature = "ui", allow(dead_code))]
    None,
}

impl StaticRoot {
    /// Chọn nguồn file tĩnh theo cấu hình; quyền thắng thuộc `[web].ui_dir`.
    pub(super) fn from_config(config: &Config) -> Self {
        match config.web_ui_dir() {
            Some(dir) => Self::Dir(dir.to_path_buf()),
            None => {
                #[cfg(feature = "ui")]
                {
                    Self::Embedded
                }
                #[cfg(not(feature = "ui"))]
                {
                    Self::None
                }
            }
        }
    }
}

/// Layer gắn `Cache-Control` cố định — dùng cho UI phục vụ từ `[web].ui_dir`.
///
/// Giữ cùng bất biến với bản nhúng (mục 11.3): `assets/*` có hash trong tên nên
/// cache vĩnh viễn an toàn; `index.html` phải `no-cache` để trỏ đúng hash mới
/// sau khi người dùng chép `web/dist` mới vào `[web].ui_dir`.
pub(super) fn cache_control_layer(value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static(value))
}

pub(super) async fn ui_handler(uri: Uri) -> Response {
    if uri.path() == "/api" || uri.path().starts_with("/api/") {
        return ApiFailure::not_found().into_response();
    }
    #[cfg(feature = "ui")]
    {
        embedded_response(uri.path())
    }
    #[cfg(not(feature = "ui"))]
    {
        let body = r#"<!doctype html><html lang="vi"><meta charset="utf-8"><title>Bean</title><body><h1>Bean</h1><p>UI sẽ được phục vụ khi build web.</p></body></html>"#;
        (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response()
    }
}

#[cfg(feature = "ui")]
#[derive(rust_embed::Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist/"]
struct AssetsRoot;

#[cfg(feature = "ui")]
#[derive(rust_embed::Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist/assets/"]
#[prefix = "assets/"]
struct AssetsAssets;

#[cfg(feature = "ui")]
fn embedded_response(path: &str) -> Response {
    let relative = path.trim_start_matches('/');

    // Try to get from AssetsRoot first (for root files like index.html, theme-init.js)
    let (served_path, file) = match AssetsRoot::get(relative) {
        Some(file) => (relative, Some(file)),
        None => match AssetsAssets::get(relative) {
            Some(file) => (relative, Some(file)),
            None => ("index.html", AssetsRoot::get("index.html")),
        },
    };
    let Some(file) = file else {
        let body = r#"<!doctype html><html lang="vi"><meta charset="utf-8"><title>Bean</title><body><h1>Bean</h1><p>UI chưa được build; API vẫn hoạt động.</p></body></html>"#;
        return (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response();
    };
    let mime = mime_guess::from_path(served_path)
        .first_raw()
        .unwrap_or("application/octet-stream");
    let cache = if served_path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
        file.data,
    )
        .into_response()
}
