//! So khớp origin cho whitelist `allowed_origins` (M26).
//!
//! # Vấn đề wildcard hậu tố
//!
//! Người dùng muốn khai `*.dev.internal` để test trên mọi service nội bộ. Cách
//! "hiển thẳng" (`url.ends_with(".dev.internal")`) là **sai và nguy hiểm**: nó khớp
//! `dev.internal.attacker.com`, tức kẻ tấn công chỉ cần đăng ký một domain họ kiểm
//! soát, kết thúc bằng chuỗi đó, là vượt whitelist.
//!
//! Vì vậy [`OriginPattern::host_matches`] so khớp **theo ranh giới label DNS**: phần
//! đuôi phải khớp và ký tự ngay trước phần đuôi **bắt buộc** là `.`.
//!
//! # Fail-closed
//!
//! * `allowed_origins` rỗng ⇒ **không** khớp gì ⇒ mọi origin ngoài whitelist đều
//!   `Dangerous`. Rỗng không bao giờ có nghĩa "cho phép tất cả".
//! * Scheme khớp **chính xác**: whitelist có `https://dev.internal` thì
//!   `http://dev.internal` vẫn ngoài whitelist. Không bao giờ hạ scheme.
//! * Port khớp **chính xác** khi mẫu ghi port; không ghi thì chỉ khớp port mặc định
//!   của scheme (80/443) — không phải "bất kỳ port nào".

use std::net::Ipv4Addr;
use std::net::Ipv6Addr;

/// Kết quả kiểm tra origin so với whitelist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginVerdict {
    /// Origin nằm trong `allowed_origins` ⇒ tool hành động chạy `Confirm`
    /// (có tuỳ chọn "cho phép trong phiên").
    Whitelisted,
    /// Origin **không** nằm trong whitelist ⇒ `Dangerous`, không session-wide.
    Outside,
}

/// Một mục trong `allowed_origins`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginPattern {
    /// Scheme đã hạ chữ thường (`http`/`https`).
    scheme: String,
    /// Host đã hạ chữ thường; có thể bắt đầu bằng nhãn wildcard `*`.
    host: String,
    /// Port ghi trong mẫu, `None` nghĩa là "chỉ port mặc định của scheme".
    port: Option<u16>,
}

impl OriginPattern {
    /// Port mặc định của một scheme.
    fn default_port(scheme: &str) -> u16 {
        if scheme == "https" { 443 } else { 80 }
    }

    /// Mẫu này có chứa wildcard không (`*.dev.internal`).
    #[must_use]
    pub fn has_wildcard(&self) -> bool {
        self.host.starts_with('*')
    }

    /// Mẫu này khớp origin cho trước hay không.
    #[must_use]
    pub fn matches(&self, scheme: &str, host: &str, port: u16) -> bool {
        // 1. Scheme khớp tuyệt đối — không bao giờ cho phép hạ từ https xuống http.
        if !self.scheme.eq_ignore_ascii_case(scheme) {
            return false;
        }
        // 2. Port: mẫu ghi port thì phải khớp; không ghi thì chỉ port mặc định.
        let expected_port = self
            .port
            .unwrap_or_else(|| Self::default_port(&self.scheme));
        if port != expected_port {
            return false;
        }
        self.host_matches(host)
    }

    /// So khớp host, xử lý wildcard **theo ranh giới label**.
    fn host_matches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        if !self.has_wildcard() {
            return self.host == host;
        }

        // `*.dev.internal` ⇒ phần đuôi bắt buộc là `dev.internal` (không kèm `*.`).
        let suffix = self.host.trim_start_matches('*').trim_start_matches('.');
        if suffix.is_empty() {
            // Mẫu `*` trần khớp mọi host — **cố ý từ chối**: quá rộng, và một lỗi gõ
            // `*` trong cấu hình sẽ biến thành "mọi trang trên internet".
            return false;
        }

        // Host phải kết thúc bằng đúng phần đuôi.
        let Some(head) = host.strip_suffix(suffix) else {
            return false;
        };
        if head.is_empty() {
            // `dev.internal` trần không khớp `*.dev.internal` — wildcard cần ít nhất
            // một label phía trước, đúng như hành vi DNS.
            return false;
        }
        // Dòng **quan trọng nhất** của cả module: ký tự ngay trước phần đuôi bắt buộc
        // là `.`. Không có dòng này, `dev.internal.attacker.com` sẽ lọt vì hậu tố
        // vẫn trùng.
        head.ends_with('.')
    }
}
/// Một origin đã phân tích từ URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// Scheme đã hạ chữ thường.
    pub scheme: String,
    /// Host đã hạ chữ thường.
    pub host: String,
    /// Port thực tế (đã áp default theo scheme nếu URL không ghi).
    pub port: u16,
}

impl Origin {
    /// Dạng `scheme://host:port` để hiển thị trong prompt xác nhận.
    #[must_use]
    pub fn as_display(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }

    /// Host có phải là địa chỉ IP viết thẳng không (thay vì tên miền)?
    #[must_use]
    pub fn host_is_ip_literal(&self) -> bool {
        self.host.parse::<std::net::IpAddr>().is_ok()
    }

    /// IP literal nếu host là IP, `None` nếu là tên miền.
    #[must_use]
    pub fn ip_literal(&self) -> Option<std::net::IpAddr> {
        self.host.parse::<std::net::IpAddr>().ok()
    }

    /// Host là loopback không (`127.0.0.0/8`, `::1`, `localhost`)?
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        match self.ip_literal() {
            Some(std::net::IpAddr::V4(v4)) => v4.is_loopback(),
            Some(std::net::IpAddr::V6(v6)) => v6.is_loopback(),
            None => self.host == "localhost",
        }
    }
}

/// Parse một URL thành [`Origin`], từ chối scheme/credential/host rỗng.
#[must_use]
pub fn parse_origin(url: &url::Url) -> Option<Origin> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // Credential nhúng trong URL là đường rò cookie sang origin bên thứ ba.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    // `host_str` đã bỏ dấu ngoặc vuông của IPv6 literal; bỏ nốt cho chắc ăn.
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = url
        .port()
        .unwrap_or_else(|| if url.scheme() == "https" { 443 } else { 80 });
    Some(Origin {
        scheme: url.scheme().to_ascii_lowercase(),
        host,
        port,
    })
}

/// Danh sách mẫu đã parse, dùng để tra nhanh nhiều origin.
#[derive(Debug, Clone, Default)]
pub struct OriginWhitelist {
    patterns: Vec<OriginPattern>,
}

impl OriginWhitelist {
    /// Dựng whitelist từ các chuỗi trong cấu hình.
    ///
    /// Mục **không parse được** bị bỏ kèm cảnh báo — nhưng `Config::validate` đã chặn
    /// trước, nên ở đây còn sót là bất thường.
    #[must_use]
    pub fn parse(raw: &[String]) -> Self {
        let patterns = raw
            .iter()
            .filter_map(|item| match parse_pattern(item) {
                Ok(pattern) => Some(pattern),
                Err(reason) => {
                    tracing::warn!(pattern = %item, reason = %reason, "bỏ qua mẫu origin không hợp lệ");
                    None
                }
            })
            .collect();
        Self { patterns }
    }

    /// Whitelist có mục nào chứa wildcard không (dùng để log cảnh báo).
    #[must_use]
    pub fn has_wildcard(&self) -> bool {
        self.patterns.iter().any(OriginPattern::has_wildcard)
    }

    /// Số mẫu đang có.
    #[must_use]
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Whitelist rỗng không?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Phán quyết cho một origin. Rỗng ⇒ [`OriginVerdict::Outside`] (fail-closed).
    #[must_use]
    pub fn verdict(&self, origin: &Origin) -> OriginVerdict {
        let matched = self
            .patterns
            .iter()
            .any(|pattern| pattern.matches(&origin.scheme, &origin.host, origin.port));
        if matched {
            OriginVerdict::Whitelisted
        } else {
            OriginVerdict::Outside
        }
    }
}

/// Parse một chuỗi `allowed_origins` thành [`OriginPattern]`.
///
/// Chấp nhận `scheme://host[:port]` và `scheme://*.host[:port]`. Không chấp nhận
/// path, query hay credential — origin **không** có đường dẫn.
fn parse_pattern(raw: &str) -> Result<OriginPattern, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("mẫu rỗng".into());
    }
    // Tự tách scheme thay vì dựa vào `Url::parse` (hành vi với `*` trong host không
    // được bảo đảm giữa các phiên bản `url`).
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err("thiếu scheme, phải có dạng `scheme://host[:port]`".into());
    };
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return Err(format!("scheme `{scheme}` không được phép"));
    }
    if rest.contains('/') {
        return Err("origin không được chứa đường dẫn".into());
    }
    if rest.contains('@') {
        return Err("origin không được chứa credential".into());
    }

    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => {
            let parsed: u16 = port
                .parse()
                .map_err(|_| format!("port `{port}` không phải số"))?;
            (host, Some(parsed))
        }
        None => (rest, None),
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return Err("thiếu host".into());
    }
    if host.starts_with('*') && !host.starts_with("*.") {
        return Err("wildcard chỉ được dùng dạng `*.host`".into());
    }
    if host.contains('*') && !host.starts_with("*.") {
        return Err(format!(
            "wildcard chỉ được dùng ở label đầu tiên, không phải `{host}`"
        ));
    }
    if host.matches('*').count() > 1 {
        return Err("chỉ nhận tối đa một wildcard".into());
    }
    if !host.starts_with('*') && url::Host::parse(&host).is_err() {
        return Err(format!("host `{host}` không hợp lệ"));
    }
    Ok(OriginPattern { scheme, host, port })
}

/// Địa chỉ IP private/loopback/link-local — dùng bởi [`crate::guard`].
#[must_use]
pub fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_private_v4(v4),
        std::net::IpAddr::V6(v6) => is_private_v6(v6),
    }
}

/// IPv4 nằm trong dải không được phép.
#[must_use]
pub fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
        // 100.64.0.0/10 — CGNAT: không public nhưng cũng không phải private thuần.
        || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        // 169.254.0.0/16 — gồm luôn metadata endpoint 169.254.169.254.
        || (ip.octets()[0] == 169 && ip.octets()[1] == 254)
        // 192.0.0.0/24 và 198.18.0.0/15 — dải đặc biệt.
        || (ip.octets()[0] == 192 && ip.octets()[1] == 0 && ip.octets()[2] == 0)
        || (ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
}

/// IPv6 nằm trong dải không được phép (tương ứng IPv4 + link-local + ULA).
#[must_use]
pub fn is_private_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    // fc00::/7 — Unique Local Address (tương đương IPv4 private).
    if (ip.segments()[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 — link-local (tương đương 169.254/16).
    if (ip.segments()[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // ::ffff:0:0/96 — IPv4-mapped: phải kiểm lại phần IPv4, không thì
    // `::ffff:127.0.0.1` sẽ lọt.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_private_v4(v4);
    }
    // :: — IPv4-compatible.
    if ip.segments()[..6] == [0, 0, 0, 0, 0, 0] {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use url::Url;

    fn origin(url: &str) -> Origin {
        parse_origin(&Url::parse(url).unwrap()).expect("URL hợp lệ")
    }

    fn list(items: &[&str]) -> OriginWhitelist {
        OriginWhitelist::parse(
            &items
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<String>>(),
        )
    }

    // -----------------------------------------------------------------------
    // Ranh giới label — test quyết định: không có nó thì wildcard là lỗ hổng
    // -----------------------------------------------------------------------

    #[test]
    fn wildcard_does_not_match_domain_that_merely_ends_with_suffix() {
        // Đây là lý do `ends_with` bị bỏ. `*.dev.internal` KHÔNG được khớp
        // `dev.internal.attacker.com` — kẻ tấn công chỉ cần sở hữu domain đó.
        let wl = list(&["https://*.dev.internal"]);
        for evil in [
            // Hậu tố chuỗi trùng nhưng KHÔNG phải subdomain: đây là dạng tấn công
            // thật sự, vì kẻ tấn công chỉ cần đăng ký tên miền mình kiểm soát.
            "https://dev.internal.attacker.com",
            "https://a.dev.internal.attacker.com",
            "https://evil-dev.internal.attacker.io",
            // Có hậu tố nhưng label trước bị dính chữ: `notdev` là MỘT label, không
            // phải `dev` — ranh giới `.` mới quyết định, không phải vị trí ký tự.
            "https://notdev.internal",
            "https://dev.internalx",
        ] {
            assert_eq!(
                wl.verdict(&origin(evil)),
                OriginVerdict::Outside,
                "{evil} phải nằm NGOÀI whitelist"
            );
        }
    }

    #[test]
    fn wildcard_matches_labels_under_the_suffix() {
        let wl = list(&["https://*.dev.internal"]);
        for good in [
            "https://api.dev.internal",
            "https://web-01.dev.internal",
            "https://a.b.c.dev.internal", // nhiều label phía trước vẫn hợp lệ
            // Label trước có chứa chữ của phần đuôi vẫn là một label riêng, nên
            // `xevil` KHÔNG phải `dev` bị dính chữ: đây là subdomain hợp lệ. Test này
            // chốt rằng bộ khớp phân biệt ranh giới label, chứ không "quá chặt" tới
            // mức từ chối cả subdomain thật.
            "https://xevil.dev.internal",
        ] {
            assert_eq!(
                wl.verdict(&origin(good)),
                OriginVerdict::Whitelisted,
                "{good} phải nằm TRONG whitelist"
            );
        }
    }

    #[test]
    fn wildcard_does_not_match_bare_suffix() {
        // `*.dev.internal` yêu cầu ít nhất một label phía trước — hành vi DNS.
        let wl = list(&["https://*.dev.internal"]);
        assert_eq!(
            wl.verdict(&origin("https://dev.internal")),
            OriginVerdict::Outside
        );
    }

    // -----------------------------------------------------------------------
    // Scheme và port — không bao giờ hạ scheme, port phải khớp
    // -----------------------------------------------------------------------

    #[test]
    fn scheme_must_match_exactly_and_never_downgrades() {
        let wl = list(&["https://dev.internal"]);
        assert_eq!(
            wl.verdict(&origin("https://dev.internal")),
            OriginVerdict::Whitelisted
        );
        // Hạ https xuống http là đường tấn công MITM kinh điển.
        assert_eq!(
            wl.verdict(&origin("http://dev.internal")),
            OriginVerdict::Outside
        );
    }

    #[test]
    fn port_must_match_exactly_when_specified() {
        let wl = list(&["http://localhost:3000"]);
        assert_eq!(
            wl.verdict(&origin("http://localhost:3000")),
            OriginVerdict::Whitelisted
        );
        assert_eq!(
            wl.verdict(&origin("http://localhost:4000")),
            OriginVerdict::Outside
        );
        assert_eq!(
            wl.verdict(&origin("http://localhost")),
            OriginVerdict::Outside
        );
    }

    #[test]
    fn pattern_without_port_matches_only_default_port() {
        // Không ghi port KHÔNG phải "mọi port".
        let wl = list(&["https://dev.internal"]);
        assert_eq!(
            wl.verdict(&origin("https://dev.internal")),
            OriginVerdict::Whitelisted
        );
        assert_eq!(
            wl.verdict(&origin("https://dev.internal:8443")),
            OriginVerdict::Outside
        );
    }

    // -----------------------------------------------------------------------
    // Fail-closed
    // -----------------------------------------------------------------------

    #[test]
    fn empty_whitelist_matches_nothing() {
        // Rỗng KHÔNG được nghĩa là "cho phép tất cả" — ngược lại sẽ hạ mọi origin
        // xuống Confirm và mở đường cho mọi trang trên internet.
        let wl = list(&[]);
        assert!(wl.is_empty());
        for url in [
            "http://localhost:3000",
            "https://example.com",
            "http://127.0.0.1",
        ] {
            assert_eq!(wl.verdict(&origin(url)), OriginVerdict::Outside, "{url}");
        }
    }

    #[test]
    fn bare_star_pattern_is_rejected_as_too_broad() {
        // Lỗi gõ `*` trong cấu hình không được biến thành "mọi trang".
        assert!(parse_pattern("*").is_err());
        assert!(parse_pattern("https://*").is_err());
        let wl = list(&["https://*"]);
        assert!(
            wl.is_empty(),
            "mẫu `https://*` phải bị bỏ, không phải khớp mọi thứ"
        );
    }

    #[test]
    fn wildcard_only_allowed_as_first_label() {
        assert!(parse_pattern("https://dev.*.internal").is_err());
        assert!(parse_pattern("https://de*").is_err());
        assert!(parse_pattern("https://*.*.internal").is_err());
        assert!(parse_pattern("https://*.dev.internal").is_ok());
    }

    // -----------------------------------------------------------------------
    // Parse: fail-closed với input xấu
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_patterns_are_rejected() {
        for bad in [
            "",
            "   ",
            "dev.internal",             // thiếu scheme
            "ftp://dev.internal",       // scheme không hợp lệ
            "file:///etc/passwd",       // scheme file
            "https://dev.internal/x",   // có đường dẫn
            "https://u:p@dev.internal", // có credential
            "https://",                 // thiếu host
            "https://dev.internal:abc", // port không phải số
        ] {
            assert!(parse_pattern(bad).is_err(), "`{bad}` phải bị từ chối");
        }
    }

    #[test]
    fn non_http_url_is_not_an_origin() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
        ] {
            assert!(
                parse_origin(&Url::parse(url).unwrap()).is_none(),
                "{url} không phải origin web"
            );
        }
    }

    #[test]
    fn credential_in_url_is_rejected() {
        assert!(parse_origin(&Url::parse("https://user:pw@dev.internal").unwrap()).is_none());
    }

    #[test]
    fn host_case_is_normalised() {
        // DNS không phân biệt hoa thường; nếu không hạ chữ thường thì
        // `https://DEV.Internal` sẽ lọt khỏi whitelist `https://dev.internal`.
        let wl = list(&["https://dev.internal"]);
        assert_eq!(
            wl.verdict(&origin("https://DEV.INTERNAL")),
            OriginVerdict::Whitelisted
        );
    }

    #[test]
    fn ipv6_literal_origin_parses() {
        let o = origin("http://[::1]:8080/");
        assert_eq!(o.host, "::1");
        assert_eq!(o.port, 8080);
        assert!(o.host_is_ip_literal());
    }

    // -----------------------------------------------------------------------
    // Dải IP — lớp guard dùng
    // -----------------------------------------------------------------------

    #[test]
    fn private_and_special_ipv4_ranges_are_blocked() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "255.255.255.255",
            "100.64.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "224.0.0.1",
        ] {
            let addr: Ipv4Addr = ip.parse().unwrap();
            assert!(is_private_v4(addr), "{ip} phải bị chặn");
        }
    }

    #[test]
    fn public_ipv4_is_allowed() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "203.0.113.7",
            "172.32.0.1",
            "100.128.0.1",
        ] {
            let addr: Ipv4Addr = ip.parse().unwrap();
            assert!(!is_private_v4(addr), "{ip} phải được phép");
        }
    }

    #[test]
    fn private_and_mapped_ipv6_are_blocked() {
        for ip in [
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            let addr: Ipv6Addr = ip.parse().unwrap();
            assert!(is_private_v6(addr), "{ip} phải bị chặn");
        }
        assert!(!is_private_v6("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn localhost_hostname_counts_as_loopback() {
        // `localhost` là tên, không phải IP — nhưng đi tới loopback, nên lớp guard
        // phải coi nó như loopback để không lọt qua khe "chỉ chặn IP literal".
        assert!(origin("http://localhost:3000").is_loopback());
        assert!(origin("http://127.0.0.1").is_loopback());
        assert!(origin("http://[::1]").is_loopback());
        assert!(!origin("https://example.com").is_loopback());
    }
}
