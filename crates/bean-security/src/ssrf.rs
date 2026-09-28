//! HTTP an toàn chống SSRF (agents.md mục 15.5).
//!
//! `SafeHttpClient` chỉ nhận URL `http(s)`. Địa chỉ IP viết trực tiếp bị kiểm tra trước
//! request; tên miền đi qua resolver tuỳ biển [`reqwest::dns::Resolve`], resolver này gọi
//! DNS ngay trước khi connector mở socket và **chỉ trả về IP toàn cục**. Vì reqwest dùng
//! đúng các `SocketAddr` trả về (không phân giải lần hai), kẻ tấn công không thể đổi DNS
//! giữa bước kiểm tra và bước kết nối.
//!
//! Redirect dùng policy tuỳ biển để xác thực lại scheme/host/IP ở *mọi* bước. System proxy
//! bị tắt để không né resolver, và toàn request có timeout. Body được giới hạn cả khi
//! khai báo `Content-Length` lẫn trong lúc streaming chunk.

use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use thiserror::Error;
use url::{Host, Url};

/// Số redirect tối đa của một lần fetch.
pub const MAX_REDIRECTS: usize = 5;
/// Deadline cho toàn request, gồm cả đọc body.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Timeout riêng cho bước mở kết nối.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// Trần mặc định của body tải về, trước khi chuyển HTML sang text.
pub const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// URL quá dài thường là dữ liệu lạ và có thể làm audit log phình to.
pub const MAX_URL_CHARS: usize = 4_096;

/// Lỗi có kiểm soát của HTTP client an toàn.
#[derive(Debug, Error)]
pub enum SsrfError {
    /// URL không parse được hoặc vượt giới hạn.
    #[error("URL không hợp lệ: {0}")]
    InvalidUrl(String),
    /// Chỉ `http` và `https` được phép.
    #[error("scheme `{0}` không được phép; chỉ chấp nhận http hoặc https")]
    UnsupportedScheme(String),
    /// URL phải có host.
    #[error("URL không có host")]
    MissingHost,
    /// Không gửi credential nhúng trong URL tới origin bên thứ ba.
    #[error("URL không được chứa username hoặc password")]
    EmbeddedCredentials,
    /// Host/IP bị chính sách SSRF chặn.
    #[error("mục tiêu bị chặn: {reason}")]
    BlockedTarget {
        /// Mô tả không chứa secret.
        reason: String,
    },
    /// DNS không còn địa chỉ public nào sau khi lọc.
    #[error("DNS `{host}` không trả về địa chỉ public an toàn")]
    NoPublicAddress {
        /// Host không tin cậy, chỉ dùng để báo lỗi.
        host: String,
    },
    /// DNS thất bại.
    #[error("không phân giải được DNS `{host}`: {message}")]
    DnsLookup {
        /// Host không tin cậy.
        host: String,
        /// Thông báo từ resolver hệ điều hành.
        message: String,
    },
    /// Chuỗi redirect vượt giới hạn.
    #[error("quá nhiều redirect (tối đa {max})")]
    TooManyRedirects {
        /// Giới hạn đã cấu hình.
        max: usize,
    },
    /// Server trả HTTP không thành công.
    #[error("máy chủ trả HTTP {status}")]
    HttpStatus {
        /// HTTP status; không echo URL/query vì có thể chứa token.
        status: u16,
    },
    /// Body vượt giới hạn, kể cả khi server không gửi `Content-Length`.
    #[error("body vượt giới hạn {limit} byte")]
    BodyTooLarge {
        /// Giới hạn thực tế của client.
        limit: usize,
    },
    /// Lỗi request/transport hoặc redirect policy.
    #[error("lỗi HTTP an toàn: {0}")]
    Request(#[source] reqwest::Error),
    /// Không dựng được HTTP client.
    #[error("không dựng được HTTP client an toàn: {0}")]
    Build(#[source] reqwest::Error),
}

/// Response đã đọc an toàn, dùng bởi `web_fetch`.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    /// URL cuối cùng sau redirect.
    pub url: String,
    /// Content-Type gốc, nếu server gửi.
    pub content_type: Option<String>,
    /// Body đã giới hạn theo byte.
    pub body: Vec<u8>,
}

/// Client HTTP dùng cho mọi URL do model cung cấp.
#[derive(Clone)]
pub struct SafeHttpClient {
    client: Client,
    max_body_bytes: usize,
}

impl std::fmt::Debug for SafeHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafeHttpClient")
            .field("max_body_bytes", &self.max_body_bytes)
            .finish_non_exhaustive()
    }
}

impl SafeHttpClient {
    /// Tạo client với resolver hệ điều hành, luôn lọc IP trước kết nối.
    ///
    /// # Errors
    /// Trả lỗi nếu thư viện HTTP không dựng được client.
    pub fn new() -> Result<Self, SsrfError> {
        Self::with_resolver(Arc::new(SystemDnsResolver), DEFAULT_MAX_BODY_BYTES)
    }

    fn with_resolver(resolver: Arc<dyn Resolve>, max_body_bytes: usize) -> Result<Self, SsrfError> {
        let client = Client::builder()
            .dns_resolver(PublicDnsResolver { resolver })
            .redirect(redirect_policy(MAX_REDIRECTS))
            .referer(false)
            .no_proxy()
            .user_agent(concat!("bean/", env!("CARGO_PKG_VERSION")))
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(SsrfError::Build)?;
        Ok(Self {
            client,
            max_body_bytes,
        })
    }

    /// Tải một URL và đọc body trong giới hạn.
    ///
    /// # Errors
    /// Trả lỗi URL/SSRF, HTTP, timeout hoặc body vượt giới hạn.
    pub async fn fetch(&self, raw_url: &str) -> Result<FetchedPage, SsrfError> {
        let url = validate_url(raw_url)?;
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(map_request_error)?;
        let final_url = response.url().clone();
        // Redirect policy đã kiểm tra từng URL; kiểm tra lần cuối cũng bảo đảm caller khác
        // không thể tự bỏ qua policy bằng cách sửa đối tượng response.
        validate_url(final_url.as_str())?;
        let status = response.status();
        if !status.is_success() {
            return Err(SsrfError::HttpStatus {
                status: status.as_u16(),
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_body_limited(&mut response, self.max_body_bytes).await?;
        Ok(FetchedPage {
            url: final_url.to_string(),
            content_type,
            body,
        })
    }

    /// Client chỉ dùng trong unit test của crate, ép một host giả về server local.
    #[cfg(test)]
    pub(crate) fn for_test(
        server_addr: std::net::SocketAddr,
        timeout: Duration,
        max_body_bytes: usize,
    ) -> Result<Self, SsrfError> {
        let client = Client::builder()
            .resolve_to_addrs("public.test", &[server_addr])
            .redirect(redirect_policy(MAX_REDIRECTS))
            .referer(false)
            .no_proxy()
            .timeout(timeout)
            .build()
            .map_err(SsrfError::Build)?;
        Ok(Self {
            client,
            max_body_bytes,
        })
    }

    /// Tải một URL kèm header `Authorization: Bearer <token>` (M22a — billing read-only).
    ///
    /// Dùng cho endpoint billing vì **mọi** cloud provider đều xác thực kiểu bearer, và
    /// endpoint lấy từ **cấu hình** chứ không phải do model sinh ra.
    ///
    /// Bất biến an toàn:
    /// * URL vẫn qua đúng [`validate_url`] và resolver chống SSRF như [`Self::fetch`] — không
    ///   có đường vòng nào bỏ qua được.
    /// * `token` là [`SecretString`]; **không bao giờ** đưa vào `Display`/`Debug`/log. Nó chỉ
    ///   sống trong header của request, và redirect cũng bị policy kiểm lại từng bước nên
    ///   header không bị đưa sang origin khác một cách âm thầm (xem [`redirect_policy`]).
    ///
    /// # Errors
    /// Như [`Self::fetch`].
    pub async fn fetch_bearer(
        &self,
        raw_url: &str,
        token: &SecretString,
    ) -> Result<FetchedPage, SsrfError> {
        let url = validate_url(raw_url)?;
        let mut response = self
            .client
            .get(url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(map_request_error)?;
        let final_url = response.url().clone();
        validate_url(final_url.as_str())?;
        let status = response.status();
        if !status.is_success() {
            return Err(SsrfError::HttpStatus {
                status: status.as_u16(),
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_body_limited(&mut response, self.max_body_bytes).await?;
        Ok(FetchedPage {
            url: final_url.to_string(),
            content_type,
            body,
        })
    }

    /// `POST` JSON kèm `Authorization: Bearer` (M24 — `marketing_publish`).
    ///
    /// Dùng cho endpoint lấy từ **cấu hình** (không phải do model sinh ra) với credential
    /// quyền tối thiểu "chỉ post". Mọi bảo đảm của [`Self::fetch_bearer`] giữ nguyên:
    /// URL qua [`validate_url`], resolver chống SSRF, redirect kiểm lại từ bước, body giới
    /// hạn, và `token` là [`SecretString`] không bao giờ vào log.
    ///
    /// # Errors
    /// Như [`Self::fetch`].
    pub async fn post_bearer(
        &self,
        raw_url: &str,
        payload: &Value,
        token: &SecretString,
    ) -> Result<FetchedPage, SsrfError> {
        let url = validate_url(raw_url)?;
        let mut response = self
            .client
            .post(url)
            .bearer_auth(token.expose_secret())
            .json(payload)
            .send()
            .await
            .map_err(map_request_error)?;
        let final_url = response.url().clone();
        validate_url(final_url.as_str())?;
        let status = response.status();
        if !status.is_success() {
            return Err(SsrfError::HttpStatus {
                status: status.as_u16(),
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_body_limited(&mut response, self.max_body_bytes).await?;
        Ok(FetchedPage {
            url: final_url.to_string(),
            content_type,
            body,
        })
    }
}

/// Parse và kiểm tra URL trước khi gửi request.
///
/// # Errors
/// Trả lỗi nếu URL sai, không phải HTTP(S), chứa credential, thiếu host, hoặc host/IP
/// vi phạm chính sách mạng.
pub fn validate_url(raw_url: &str) -> Result<Url, SsrfError> {
    if raw_url.chars().count() > MAX_URL_CHARS {
        return Err(SsrfError::InvalidUrl(format!(
            "URL dài hơn {MAX_URL_CHARS} ký tự"
        )));
    }
    let url = Url::parse(raw_url).map_err(|err| SsrfError::InvalidUrl(err.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SsrfError::UnsupportedScheme(url.scheme().to_owned()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SsrfError::EmbeddedCredentials);
    }
    let host = url.host().ok_or(SsrfError::MissingHost)?.to_owned();
    match host {
        Host::Ipv4(ip) => ensure_public_ip(IpAddr::V4(ip))?,
        Host::Ipv6(ip) => ensure_public_ip(IpAddr::V6(ip))?,
        Host::Domain(domain) => {
            let normalized = domain.trim_end_matches('.').to_ascii_lowercase();
            if normalized == "localhost" || normalized.ends_with(".localhost") {
                return Err(SsrfError::BlockedTarget {
                    reason: format!("host local `{normalized}` bị chặn"),
                });
            }
        }
    }
    Ok(url)
}

/// Chặn mọi địa chỉ không phải global unicast.
///
/// IPv4 dùng danh sách CIDR đặc biệt cụ thể; IPv6 chỉ cho phép `2000::/3`, sau đó loại
/// các dải đặc biệt phổ biến (Teredo, 6to4, benchmark, documentation, ORCHID).
fn ensure_public_ip(ip: IpAddr) -> Result<(), SsrfError> {
    let allowed = match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    };
    if allowed {
        Ok(())
    } else {
        Err(SsrfError::BlockedTarget {
            reason: format!("địa chỉ `{ip}` không thuộc mạng public"),
        })
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    // 0/8, 10/8, 100.64/10, 127/8, 169.254/16, 172.16/12, các dải đặc biệt,
    // multicast và reserved đều không phải public unicast.
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    // IPv4-compatible ::a.b.c.d đã deprecated; không cho phép để tránh cách biểu diễn khác.
    if ip.to_ipv4().is_some() {
        return false;
    }
    let segment = ip.segments();
    // Global unicast hiện nằm trong 2000::/3. Ngoài ra loại các dải đặc biệt có thể che
    // địa chỉ IPv4 hoặc không định tuyến trực tiếp.
    (segment[0] & 0xe000) == 0x2000
        && !(segment[0] == 0x2001 && segment[1] == 0x0000) // Teredo 2001::/32
        && !(segment[0] == 0x2001 && segment[1] == 0x0002) // benchmarking 2001:2::/48
        && !(segment[0] == 0x2001 && (segment[1] & 0xfff0) == 0x0010) // ORCHID 2001:10::/28
        && !(segment[0] == 0x2001 && (segment[1] & 0xfff0) == 0x0020) // ORCHIDv2 2001:20::/28
        && !(segment[0] == 0x2001 && segment[1] == 0x0db8) // documentation
        && !(segment[0] == 0x2002) // 6to4 2002::/16
        && !(segment[0] == 0x3ffe) // 6bone đã lỗi thời
        && !(segment[0] == 0x3fff && (segment[1] & 0xf000) == 0) // documentation 3fff::/20
}

/// Resolver mặc định, gọi resolver của hệ điều hành qua Tokio.
#[derive(Debug, Default)]
struct SystemDnsResolver;

impl Resolve for SystemDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|err| {
                    Box::new(SsrfError::DnsLookup {
                        host: host.clone(),
                        message: err.to_string(),
                    }) as Box<dyn Error + Send + Sync>
                })?
                .collect::<Vec<_>>();
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// Resolver bắt buộc của reqwest: mọi IP trả về đều phải là public unicast.
struct PublicDnsResolver {
    resolver: Arc<dyn Resolve>,
}

impl std::fmt::Debug for PublicDnsResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublicDnsResolver")
    }
}

impl Resolve for PublicDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let resolver = Arc::clone(&self.resolver);
        Box::pin(async move {
            let resolved = resolver.resolve(name).await?;
            let mut public = Vec::new();
            for address in resolved {
                if let Err(err) = ensure_public_ip(address.ip()) {
                    return Err(Box::new(err) as Box<dyn Error + Send + Sync>);
                }
                public.push(address);
            }
            if public.is_empty() {
                return Err(Box::new(SsrfError::NoPublicAddress { host }) as _);
            }
            Ok(Box::new(public.into_iter()) as Addrs)
        })
    }
}

/// Chính sách redirect kiểm tra lại toàn bộ URL đích trước khi reqwest theo.
fn redirect_policy(max: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > max {
            return attempt.error(SsrfError::TooManyRedirects { max });
        }
        match validate_url(attempt.url().as_str()) {
            Ok(_) => attempt.follow(),
            Err(err) => attempt.error(err),
        }
    })
}

/// Đọc response theo chunk, dừng ngay khi vượt trần; không dùng `.bytes()` không giới hạn.
pub(crate) async fn read_body_limited(
    response: &mut reqwest::Response,
    max_body_bytes: usize,
) -> Result<Vec<u8>, SsrfError> {
    if let Some(length) = response.content_length()
        && length > u64::try_from(max_body_bytes).unwrap_or(u64::MAX)
    {
        return Err(SsrfError::BodyTooLarge {
            limit: max_body_bytes,
        });
    }
    let mut body = Vec::with_capacity(max_body_bytes.min(64 * 1024));
    while let Some(chunk) = response.chunk().await.map_err(map_request_error)? {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or(SsrfError::BodyTooLarge {
                limit: max_body_bytes,
            })?;
        if next_len > max_body_bytes {
            return Err(SsrfError::BodyTooLarge {
                limit: max_body_bytes,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Giữ lại thông báo cụ thể từ redirect/resolver nếu reqwest đã bọc trong `Error::source`.
fn map_request_error(error: reqwest::Error) -> SsrfError {
    let mut source: Option<&(dyn Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(inner) = current.downcast_ref::<SsrfError>() {
            match inner {
                SsrfError::InvalidUrl(message) => {
                    return SsrfError::InvalidUrl(message.clone());
                }
                SsrfError::UnsupportedScheme(scheme) => {
                    return SsrfError::UnsupportedScheme(scheme.clone());
                }
                SsrfError::MissingHost => return SsrfError::MissingHost,
                SsrfError::EmbeddedCredentials => return SsrfError::EmbeddedCredentials,
                SsrfError::BlockedTarget { reason } => {
                    return SsrfError::BlockedTarget {
                        reason: reason.clone(),
                    };
                }
                SsrfError::NoPublicAddress { host } => {
                    return SsrfError::NoPublicAddress { host: host.clone() };
                }
                SsrfError::DnsLookup { host, message } => {
                    return SsrfError::DnsLookup {
                        host: host.clone(),
                        message: message.clone(),
                    };
                }
                SsrfError::TooManyRedirects { max } => {
                    return SsrfError::TooManyRedirects { max: *max };
                }
                _ => {}
            }
        }
        source = current.source();
    }
    // Bỏ phần URL cuối của reqwest khỏi thông báo để tránh vô tình đưa query token vào log.
    SsrfError::Request(error.without_url())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use reqwest::dns::{Addrs, Name, Resolve, Resolving};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[derive(Debug)]
    struct StaticResolver(Vec<SocketAddr>);

    impl Resolve for StaticResolver {
        fn resolve(&self, _name: Name) -> Resolving {
            let addresses = self.0.clone();
            Box::pin(async move { Ok(Box::new(addresses.into_iter()) as Addrs) })
        }
    }

    #[test]
    fn blocks_non_http_and_private_literal_targets() {
        for url in [
            "file:///etc/passwd",
            "http://127.0.0.1/",
            "http://localhost/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            let error = validate_url(url).unwrap_err();
            assert!(
                matches!(
                    error,
                    SsrfError::UnsupportedScheme(_) | SsrfError::BlockedTarget { .. }
                ),
                "{url} phải bị chặn, nhận {error:?}"
            );
        }
    }

    #[tokio::test]
    async fn resolver_blocks_domain_that_points_to_private_ip() {
        let private = SocketAddr::from(([10, 0, 0, 1], 80));
        let client = SafeHttpClient::with_resolver(
            Arc::new(StaticResolver(vec![private])),
            DEFAULT_MAX_BODY_BYTES,
        )
        .unwrap();
        let error = client.fetch("http://internal.test/").await.unwrap_err();
        assert!(
            matches!(error, SsrfError::BlockedTarget { .. }),
            "resolver phải chặn trước socket: {error:?}"
        );
    }

    #[tokio::test]
    async fn redirect_to_private_ip_is_rejected_before_second_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", "http://169.254.169.254/latest/meta-data/"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/latest/meta-data/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
            .expect(0)
            .mount(&server)
            .await;

        let client =
            SafeHttpClient::for_test(*server.address(), Duration::from_secs(2), 1024).unwrap();
        let error = client.fetch("http://public.test/start").await.unwrap_err();
        assert!(error.to_string().contains("bị chặn"), "{error:?}");
    }

    #[tokio::test]
    async fn body_limit_and_timeout_are_enforced() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/large"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(100)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("late")
                    .set_delay(Duration::from_millis(200)),
            )
            .mount(&server)
            .await;
        let client =
            SafeHttpClient::for_test(*server.address(), Duration::from_millis(40), 32).unwrap();

        let error = client.fetch("http://public.test/large").await.unwrap_err();
        assert!(matches!(error, SsrfError::BodyTooLarge { limit: 32 }));

        let started = Instant::now();
        let error = client.fetch("http://public.test/slow").await.unwrap_err();
        assert!(matches!(error, SsrfError::Request(_)), "{error:?}");
        assert!(started.elapsed() < Duration::from_millis(150));
    }
}
