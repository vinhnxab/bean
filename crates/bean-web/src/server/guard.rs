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
//!
//! # Ngoai le IP literal (D21.6)
//!
//! Nhoi `Origin` khop dung `Host` cua request va Host la **IP literal** (loopback/RFC1918)
//! cung duoc chap nhan — truy cap thang qua `http://127.0.0.1:7878` khi `public_origin`
//! la IP LAN. DNS rebinding bat buoc dung domain nen khong ma duoc Host dang IP; trang
//! tan cong o IP/port khac luon co `Origin != Host` (Origin do browser dat, khong gia duoc).
//!
//! # Sau reverse proxy (`web.trust_proxy = true`, D4.3)
//!
//! Proxy tin cay ghi de `Host` thanh dia chi noi bo (vi du proxy cua InstaBox gui
//! `Host: 127.0.0.1:7890` kem `X-Forwarded-Host: <domain-cong-khai>`), khi đó Host goc
//! duoc doc tu `X-Forwarded-Host`/`X-Forwarded-Port`. Van an toan vi nua con lai cua cap
//! check van bat buoc `Origin` khop `public_origin` — Origin la header browser khong the
//! gia mao, nen `X-Forwarded-Host` (browser co the gui tu ky) mot minh khong mo duoc duong.

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use url::Url;

use super::{ApiFailure, WebState};

const X_FORWARDED_HOST: header::HeaderName = header::HeaderName::from_static("x-forwarded-host");
const X_FORWARDED_PORT: header::HeaderName = header::HeaderName::from_static("x-forwarded-port");

/// Origin/Host/CSRF middleware cho mọi request thay đổi dữ liệu và WebSocket.
pub async fn request_guard(
    State(state): State<WebState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let headers = request.headers().clone();
    let trust_proxy = state.config.web.trust_proxy;
    if path.starts_with("/api/") && is_mutating(method) {
        if !same_site_allowed(&headers, &state.public_origin(), trust_proxy) {
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
    if path == "/api/ws" && !same_site_allowed(&headers, &state.public_origin(), trust_proxy) {
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
        // `media-src 'self' blob:` (M27): TTS trả audio rồi phát bằng object URL của
        // `Blob` (blob: cùng nguồn gốc). Không nới `script-src`/`default-src`; `data:`
        // cho media vẫn bị chặn nên ảnh/âm thanh nhúng dữ liệu không lọt qua được.
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; style-src 'self' 'unsafe-inline'",
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

/// Host hiệu lực của request. Khi `web.trust_proxy = true` (sau reverse proxy tin cậy,
/// D4.3) proxy ghi đè `Host` thành địa chỉ nội bộ — đọc `X-Forwarded-Host` thay thế,
/// port lấy từ `X-Forwarded-Port` nếu proxy gửi riêng, thiếu XFH thì fallback về `Host`
/// như cũ. Cặp check vẫn bắt buộc `Origin` khớp `public_origin` ở caller nên
/// `X-Forwarded-Host` (browser gửi tùy ý được) một mình không qua được guard.
fn host_matches(headers: &HeaderMap, origin: &Url, trust_proxy: bool) -> bool {
    let Some((host, port)) = effective_authority(headers, trust_proxy) else {
        return false;
    };
    let expected = origin.host_str().unwrap_or_default();
    let expected_port = origin.port_or_known_default();
    let actual_port = port.or_else(|| {
        if origin.scheme() == "https" {
            Some(443)
        } else {
            Some(80)
        }
    });
    host.eq_ignore_ascii_case(expected) && actual_port == expected_port
}

/// Authority hiệu lực dưới dạng `(host, port tùy chọn)`.
fn effective_authority(headers: &HeaderMap, trust_proxy: bool) -> Option<(String, Option<u16>)> {
    if trust_proxy && let Some(authority) = forwarded_authority(headers) {
        return Some(authority);
    }
    let host = header_str(headers, header::HOST)?;
    let authority = host.parse::<Authority>().ok()?;
    Some((authority.host().to_string(), authority.port_u16()))
}

/// Phần tử ĐẦU của `X-Forwarded-Host` (Host gốc phía client nhìn thấy); port ưu tiên
/// `X-Forwarded-Port`, thiếu thì lấy trong chính authority.
fn forwarded_authority(headers: &HeaderMap) -> Option<(String, Option<u16>)> {
    let forwarded_host = header_str(headers, X_FORWARDED_HOST)?;
    let first = forwarded_host.split(',').next()?.trim();
    let authority = first.parse::<Authority>().ok()?;
    let port = header_str(headers, X_FORWARDED_PORT)
        .and_then(|values| values.split(',').next())
        .and_then(|value| value.trim().parse::<u16>().ok())
        .or(authority.port_u16());
    Some((authority.host().to_string(), port))
}

fn origin_matches(headers: &HeaderMap, origin: &Url) -> bool {
    header_str(headers, header::ORIGIN)
        .is_some_and(|value| value == origin.as_str().trim_end_matches('/'))
}

/// Chấp nhận khi cặp check chuẩn khớp, hoặc khi `Origin` khớp đúng `Host` và Host là
/// IP literal (D21.6) — cho phép toggle LAN Access qua `http://127.0.0.1` dù
/// `public_origin` đã là IP LAN.
fn same_site_allowed(headers: &HeaderMap, public_origin: &Url, trust_proxy: bool) -> bool {
    (host_matches(headers, public_origin, trust_proxy) && origin_matches(headers, public_origin))
        || origin_matches_host(headers)
}

/// `Origin` phải khớp đúng `Host` của request (scheme http/https, host IP, port) và
/// Host bắt buộc là IP literal. `Origin: null` (iframe sandbox) bị từ chối.
fn origin_matches_host(headers: &HeaderMap) -> bool {
    let Some(host) = header_str(headers, header::HOST) else {
        return false;
    };
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    // Rebinding chỉ hoạt động qua domain — Host dạng IP nghĩa là người dùng tự truy cập.
    let Some(host_ip) = parse_ip_literal(authority.host()) else {
        return false;
    };
    let Some(origin) = header_str(headers, header::ORIGIN)
        .filter(|value| !value.eq_ignore_ascii_case("null"))
        .and_then(|value| Url::parse(value).ok())
    else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    let Some(origin_ip) = parse_ip_literal(origin.host_str().unwrap_or_default()) else {
        return false;
    };
    let origin_port = origin.port_or_known_default();
    let host_port = authority.port_u16().or_else(|| {
        if origin.scheme() == "https" {
            Some(443)
        } else {
            Some(80)
        }
    });
    origin_ip == host_ip && origin_port == host_port
}

/// Parse host IPv4/IPv6; `Url`/`Authority` trả IPv6 kèm ngoặc vuông `[::1]`.
fn parse_ip_literal(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host)
        .parse::<IpAddr>()
        .ok()
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    header_str(headers, header::CONTENT_TYPE).is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // quy ước tests/*.rs

    use super::*;
    use axum::http::HeaderValue;

    /// Proxy ghi đè `Host` thành địa chỉ nội bộ như đã gặp thật trên InstaBox.
    const INTERNAL_HOST: &str = "127.0.0.1:7890";
    const PUBLIC_HOST: &str = "bean.example.com";
    const PUBLIC_ORIGIN: &str = "https://bean.example.com";

    fn origin_url() -> Url {
        Url::parse(PUBLIC_ORIGIN).unwrap()
    }

    fn make_headers(
        host: Option<&str>,
        origin: Option<&str>,
        forwarded_host: Option<&str>,
        forwarded_port: Option<&str>,
    ) -> HeaderMap {
        let mut map = HeaderMap::new();
        let mut insert = |name: header::HeaderName, value: &str| {
            map.insert(name, HeaderValue::from_str(value).unwrap());
        };
        if let Some(host) = host {
            insert(header::HOST, host);
        }
        if let Some(origin) = origin {
            insert(header::ORIGIN, origin);
        }
        if let Some(forwarded_host) = forwarded_host {
            insert(X_FORWARDED_HOST, forwarded_host);
        }
        if let Some(forwarded_port) = forwarded_port {
            insert(X_FORWARDED_PORT, forwarded_port);
        }
        map
    }

    #[test]
    fn trust_proxy_false_bo_qua_forwarded_host() {
        // Đúng tình huống InstaBox: proxy ghi đè Host, gửi XFH đúng — vẫn phải 403 khi
        // trust_proxy = false (hành vi cũ giữ nguyên).
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some(PUBLIC_ORIGIN),
            Some(PUBLIC_HOST),
            Some("443"),
        );
        assert!(!same_site_allowed(&headers, &origin_url(), false));
    }

    #[test]
    fn trust_proxy_true_chap_nhan_forwarded_host() {
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some(PUBLIC_ORIGIN),
            Some(PUBLIC_HOST),
            Some("443"),
        );
        assert!(same_site_allowed(&headers, &origin_url(), true));
    }

    #[test]
    fn trust_proxy_true_khong_co_xfp_lay_port_mac_dinh_theo_scheme() {
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some(PUBLIC_ORIGIN),
            Some(PUBLIC_HOST),
            None,
        );
        assert!(same_site_allowed(&headers, &origin_url(), true));
    }

    #[test]
    fn trust_proxy_true_khong_co_xfh_fallback_ve_host() {
        // Truy cập thẳng vẫn hoạt động cả khi trust_proxy bật.
        let headers = make_headers(Some(PUBLIC_HOST), Some(PUBLIC_ORIGIN), None, None);
        assert!(same_site_allowed(&headers, &origin_url(), true));
        assert!(same_site_allowed(&headers, &origin_url(), false));
    }

    #[test]
    fn forwarded_host_khac_doi_bat_nhay_ke_cua_origin_dung() {
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some(PUBLIC_ORIGIN),
            Some("evil.example.com"),
            Some("443"),
        );
        assert!(!same_site_allowed(&headers, &origin_url(), true));
    }

    #[test]
    fn origin_doi_bat_nhay_ke_cua_forwarded_host_dung() {
        // Browser không giả mạo được Origin, nhưng client ngoài browser có thể đặt cả
        // hai header — vẫn phải bị chặn vì Origin không khớp public_origin.
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some("https://evil.example.com"),
            Some(PUBLIC_HOST),
            Some("443"),
        );
        assert!(!same_site_allowed(&headers, &origin_url(), true));
    }

    #[test]
    fn forwarded_port_non_chuan_khop_origin_cua_port_do() {
        let origin = Url::parse("https://bean.example.com:8443").unwrap();
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some("https://bean.example.com:8443"),
            Some(PUBLIC_HOST),
            Some("8443"),
        );
        assert!(same_site_allowed(&headers, &origin, true));
        // Port khác origin thì phải từ chối.
        let headers_sai_port = make_headers(
            Some(INTERNAL_HOST),
            Some("https://bean.example.com:8443"),
            Some(PUBLIC_HOST),
            Some("9000"),
        );
        assert!(!same_site_allowed(&headers_sai_port, &origin, true));
    }

    #[test]
    fn chuoi_forwarded_lay_phan_tu_dau_tien() {
        let headers = make_headers(
            Some(INTERNAL_HOST),
            Some(PUBLIC_ORIGIN),
            Some("bean.example.com, internal.example.com"),
            Some("443, 80"),
        );
        assert!(same_site_allowed(&headers, &origin_url(), true));
    }
}
