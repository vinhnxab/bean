//! Middleware bao mat: kiem tra `Origin`/`Host`/CSRF va security headers (muc 15.7).
//!
//! # Vai tro
//!
//! Day la **lop phong thu dau tien** cua web server, va la noi mot lo rui ro rat lon:
//! agent co quyen chay lenh nen mot trang web khac dieu khien duoc Bean thong qua
//! trinh duyet cua chinh nguoi dung (muc 22.15).
//!
//! # Vi sao tach rieng
//!
//! Kiem tra `Origin` la thu **khong the bo qua** va la thu doc doc lap, tach ra day de
//! kiem chung rang buoc "moi request thay doi du lieu deu phai qua day" chi mot lan.

use axum::extract::{Request, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use url::Url;

use super::{ApiFailure, WebState};
/// Origin/Host/CSRF middleware cho mọi request thay đổi dữ liệu và WebSocket.
pub async fn request_guard(
    State(state): State<WebState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let headers = request.headers().clone();
    if path.starts_with("/api/") && is_mutating(method) {
        if !host_matches(&headers, &state.public_origin)
            || !origin_matches(&headers, &state.public_origin)
        {
            return ApiFailure::forbidden().into_response();
        }
        if !has_json_content_type(&headers) {
            return ApiFailure::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "json_required",
                "Content-Type phải là application/json",
            )
            .into_response();
        }
    }
    if path == "/api/ws"
        && (!host_matches(&headers, &state.public_origin)
            || !origin_matches(&headers, &state.public_origin))
    {
        return ApiFailure::forbidden().into_response();
    }
    next.run(request).await
}

/// Thêm security headers sau khi handler xử lý xong.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; style-src 'self' 'unsafe-inline'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if path == "/api" || path.starts_with("/api/") {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

fn is_mutating(method: Method) -> bool {
    matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn host_matches(headers: &HeaderMap, origin: &Url) -> bool {
    let Some(host) = header_str(headers, header::HOST) else {
        return false;
    };
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    let expected = origin.host_str().unwrap_or_default();
    let expected_port = origin.port_or_known_default();
    let actual_port = authority.port_u16().or_else(|| {
        if origin.scheme() == "https" {
            Some(443)
        } else {
            Some(80)
        }
    });
    authority.host().eq_ignore_ascii_case(expected) && actual_port == expected_port
}

fn origin_matches(headers: &HeaderMap, origin: &Url) -> bool {
    header_str(headers, header::ORIGIN)
        .is_some_and(|value| value == origin.as_str().trim_end_matches('/'))
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    header_str(headers, header::CONTENT_TYPE).is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    })
}
