//! Phuc vu UI nhung + SPA fallback (feature `ui`).
//!
//! # Bat bien
//!
//! File co hash trong ten (`assets/*`) cache lau; `index.html` `no-cache` (muc 11.3).
//! Va nhu `routes.rs`: `/api/*` khong ton tai phai tra 404 JSON chu khong phai HTML.

use axum::http::{Uri, header};
use axum::response::{IntoResponse, Response};

use super::ApiFailure;
pub(super) async fn api_not_found() -> Response {
    ApiFailure::not_found().into_response()
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
#[allow_missing = true]
struct Assets;

#[cfg(feature = "ui")]
fn embedded_response(path: &str) -> Response {
    let relative = path.trim_start_matches('/');
    let (served_path, file) = match Assets::get(relative) {
        Some(file) => (relative, Some(file)),
        None => ("index.html", Assets::get("index.html")),
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
