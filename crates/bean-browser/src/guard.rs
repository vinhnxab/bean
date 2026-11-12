//! Lớp kiểm tra URL **riêng của domain browser** (M26).
//!
//! # Vì sao KHÔNG tái dùng `bean_security::ssrf`
//!
//! `web_fetch` đi qua [`SafeHttpClient`](bean_security::SafeHttpClient) với
//! resolver DNS tuỳ biến lọc IP **ngay lúc kết nối** — đó là chống DNS rebinding
//! đúng (mục 15.5). Nhưng lớp đó **cố ý chặn loopback**, còn mục đích của tool
//! browser là test ứng dụng chạy ở `http://localhost:3000`.
//!
//! Sửa `ssrf` để "ngoại lệ cho browser" sẽ làm hỏng bảo đảm của `web_fetch` — mọi
//! request của nó lại chỉ cần một `&str`. Vì vậy lớp này **tách riêng**, dùng chung
//! danh sách dải IP ([`crate::origin::is_private_ip`]) nhưng **quy tắc riêng**.
//!
//! # Quy tắc (fail-closed)
//!
//! | Tình huống | Kết quả |
//! |---|---|
//! | Origin **trong** `allowed_origins` | Cho đi — kể cả loopback. Đây là môi trường dev mà whitelist khai đích danh. |
//! | Origin ngoài whitelist, host là IP private/loopback/link-local/metadata | **Chặn cứng**. |
//! | Origin ngoài whitelist, host là tên miền | Phân giải DNS: có **bất kỳ** bản ghi nào rơi vào dải nội bộ thì **chặn cứng** (K25); phân giải thất bại thì cho đi — không có IP nào để kiểm, Chromium sẽ tự báo lỗi DNS. |
//! | scheme không phải http/https, có credential, URL quá dài | **Chặn cứng**. |
//!
//! Phân giải DNS chỉ chạy ở đường gọi thật (tên phương thức `check_resolved` —
//! async, dùng `tokio::net`); `Tool::risk` đồng bộ vẫn dùng [`BrowserGuard::check`]
//! không phân giải — đó là đánh giá mức rủi ro best-effort, không phải rào cản.
//! Rủi ro TOCTOU còn sót lại được ghi ở `docs/known-issues.md` (K25): Chromium
//! tự phân giải DNS lần nữa lúc kết nối, nên một tên miền có thể đổi IP giữa
//! lúc guard kiểm và lúc Chromium mở trang (cửa sổ cỡ TTL DNS). Lớp bù: origin
//! ngoài whitelist luôn `Dangerous` (không session-wide) và người dùng phải bấm
//! xác nhận từng lần.

use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use crate::origin::{OriginVerdict, OriginWhitelist, is_private_ip, parse_origin};

/// Lỗi khi lớp guard từ chối một URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    /// URL không parse được.
    #[error("URL không hợp lệ")]
    Malformed,
    /// Scheme không phải http/https.
    #[error("scheme `{0}` không được phép; chỉ nhận http hoặc https")]
    UnsupportedScheme(String),
    /// URL chứa credential — rò cookie sang origin bên thứ ba.
    #[error("URL không được chứa username hoặc password")]
    EmbeddedCredentials,
    /// URL dài bất thường (thường là dữ liệu rác, và phình audit log).
    #[error("URL quá dài (tối đa {MAX_URL_CHARS} ký tự)")]
    UrlTooLong,
    /// Host nằm trong dải nội bộ mà **không** có trong `allowed_origins`.
    #[error(
        "mục tiêu `{0}` nằm trong dải mạng nội bộ/loopback và KHÔNG có trong \
         [browser].allowed_origins — thêm origin đó vào cấu hình nếu đây thật sự là \
         môi trường test của bạn"
    )]
    BlockedInternalTarget(String),
}

/// Trần độ dài URL (chống phình audit log và dữ liệu rác).
pub const MAX_URL_CHARS: usize = 2_048;

/// Kết quả kiểm tra: URL được phép đi, kèm lý do để hiển thị cho người dùng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardVerdict {
    /// Origin đã kiểm, dùng cho [`OriginVerdict`] ở tầng risk.
    pub origin: crate::origin::Origin,
    /// `true` khi origin nằm trong `allowed_origins`.
    pub whitelisted: bool,
}

/// Lớp kiểm tra URL của domain browser.
#[derive(Clone)]
pub struct BrowserGuard {
    whitelist: Arc<OriginWhitelist>,
    resolver: Arc<dyn HostResolver>,
}

/// Nguồn phân giải DNS cho bước kiểm IP *đã phân giải* (K25).
///
/// Trait riêng (DIP — agents.md mục 0.10) để test inject bảng phân giải giả,
/// không cần mạng và không phụ thuộc DNS của máy chạy test.
#[async_trait]
pub trait HostResolver: Send + Sync {
    /// Phân giải host thành danh sách IP; rỗng khi không phân giải được.
    async fn resolve(&self, host: &str) -> Vec<std::net::IpAddr>;
}

/// Phân giải bằng hệ điều hành (tokio — không block runtime, mục 22.8).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemResolver;

#[async_trait]
impl HostResolver for SystemResolver {
    async fn resolve(&self, host: &str) -> Vec<std::net::IpAddr> {
        match tokio::net::lookup_host((host, 80u16)).await {
            Ok(addrs) => addrs.map(|addr| addr.ip()).collect(),
            Err(error) => {
                tracing::debug!(host, %error, "không phân giải được host khi kiểm guard browser");
                Vec::new()
            }
        }
    }
}

impl std::fmt::Debug for BrowserGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserGuard")
            .field("whitelist", &self.whitelist)
            .finish_non_exhaustive()
    }
}

impl BrowserGuard {
    /// Dựng guard từ whitelist đã parse, dùng resolver hệ điều hành.
    #[must_use]
    pub fn new(whitelist: Arc<OriginWhitelist>) -> Self {
        Self::with_resolver(whitelist, Arc::new(SystemResolver))
    }

    /// Dựng guard với resolver chỉ định (test inject bảng giả).
    #[must_use]
    pub fn with_resolver(whitelist: Arc<OriginWhitelist>, resolver: Arc<dyn HostResolver>) -> Self {
        Self {
            whitelist,
            resolver,
        }
    }

    /// Whitelist đang dùng.
    #[must_use]
    pub fn whitelist(&self) -> &OriginWhitelist {
        &self.whitelist
    }

    /// Kiểm tra một URL, trả về origin đã xác định.
    ///
    /// # Errors
    /// [`GuardError`] khi URL sai định dạng, sai scheme, có credential, quá dài, hoặc
    /// trỏ vào dải nội bộ mà không có trong whitelist.
    pub fn check(&self, raw_url: &str) -> Result<GuardVerdict, GuardError> {
        let trimmed = raw_url.trim();
        if trimmed.chars().count() > MAX_URL_CHARS {
            return Err(GuardError::UrlTooLong);
        }
        let parsed = Url::parse(trimmed).map_err(|_| GuardError::Malformed)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(GuardError::UnsupportedScheme(parsed.scheme().to_string()));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(GuardError::EmbeddedCredentials);
        }
        let origin = parse_origin(&parsed).ok_or(GuardError::Malformed)?;
        let verdict = self.whitelist.verdict(&origin);

        // Ngoại lệ loopback: CHỈ mở khi origin nằm trong whitelist. Đây là điểm
        // duy nhất `localhost` được phép — và nó phải do người dùng khai tên.
        if verdict == OriginVerdict::Outside {
            // Chặn theo **hai** điều kiện, không chỉ một:
            // 1. host là IP literal nằm trong dải bị chặn;
            // 2. host **là tên** nhưng trỏ tới loopback — `localhost` không parse ra
            //    IP nên chỉ kiểm (1) sẽ lọt qua khe "localhost không phải IP".
            // Bỏ (2) thì `allowed_origins` rỗng vẫn cho phép `http://localhost:5173`.
            let blocked_ip = origin.ip_literal().is_some_and(is_private_ip);
            let blocked_loopback_name = origin.host == "localhost" || origin.is_loopback();
            if blocked_ip || blocked_loopback_name {
                return Err(GuardError::BlockedInternalTarget(origin.as_display()));
            }
            // Tên miền ngoài whitelist trỏ về IP nội bộ *có thể* lọt; đó là khoảng
            // hở DNS rebinding đã ghi ở docs/known-issues.md (K25). Bù lại: origin đó
            // luôn Dangerous, không session-wide.
        }

        Ok(GuardVerdict {
            origin,
            whitelisted: verdict == OriginVerdict::Whitelisted,
        })
    }

    /// Kiểm tra một URL **kèm phân giải DNS** cho tên miền ngoài whitelist (K25).
    ///
    /// Đây là phương thức cho đường gọi thật (`Tool::call` — async): ngoài các
    /// kiểm tra đồng bộ của [`BrowserGuard::check`], tên miền ngoài whitelist được
    /// phân giải và chặn nếu **bất kỳ** IP nào rơi vào dải nội bộ. Origin trong
    /// whitelist bỏ qua bước này — whitelist là môi trường dev người dùng khai
    /// đích danh (localhost:3000 chẳng hạn).
    ///
    /// # Errors
    /// Như [`BrowserGuard::check`], cộng [`GuardError::BlockedInternalTarget`] khi
    /// IP đã phân giải nằm trong dải nội bộ.
    pub async fn check_resolved(&self, raw_url: &str) -> Result<GuardVerdict, GuardError> {
        let verdict = self.check(raw_url)?;
        if !verdict.whitelisted
            && verdict.origin.ip_literal().is_none()
            && verdict.origin.host != "localhost"
        {
            let ips = self.resolver.resolve(&verdict.origin.host).await;
            if ips.iter().any(|ip| is_private_ip(*ip)) {
                return Err(GuardError::BlockedInternalTarget(
                    verdict.origin.as_display(),
                ));
            }
        }
        Ok(verdict)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::origin::OriginVerdict;

    fn list(items: &[&str]) -> OriginWhitelist {
        OriginWhitelist::parse(
            &items
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<String>>(),
        )
    }

    fn guard(items: &[&str]) -> BrowserGuard {
        BrowserGuard::new(Arc::new(list(items)))
    }

    /// Resolver giả với bảng phân giải cố định — test K25 không cần mạng.
    struct FakeResolver {
        table: Vec<(&'static str, Vec<std::net::IpAddr>)>,
    }

    impl FakeResolver {
        fn single(host: &'static str, ip: std::net::IpAddr) -> Arc<Self> {
            Arc::new(Self {
                table: vec![(host, vec![ip])],
            })
        }
    }

    #[async_trait]
    impl HostResolver for FakeResolver {
        async fn resolve(&self, host: &str) -> Vec<std::net::IpAddr> {
            self.table
                .iter()
                .filter(|(name, _)| *name == host)
                .flat_map(|(_, ips)| ips.iter().copied())
                .collect()
        }
    }

    fn guard_with(items: &[&str], resolver: Arc<dyn HostResolver>) -> BrowserGuard {
        BrowserGuard::with_resolver(Arc::new(list(items)), resolver)
    }

    // -----------------------------------------------------------------------
    // K25: tên miền ngoài whitelist trỏ về IP nội bộ phải bị chặn
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rebind_to_loopback_is_blocked_by_resolved_check() {
        let g = guard_with(
            &[],
            FakeResolver::single(
                "rebind.attacker.com",
                std::net::IpAddr::from([127, 0, 0, 1]),
            ),
        );
        // `check` đồng bộ cho đi (không phân giải) — khoảng hở K25 trước đây.
        assert!(g.check("http://rebind.attacker.com/x").is_ok());
        // `check_resolved` phải chặn vì IP đã phân giải là loopback.
        let err = g
            .check_resolved("http://rebind.attacker.com/x")
            .await
            .unwrap_err();
        assert!(
            matches!(err, GuardError::BlockedInternalTarget(_)),
            "phải chặn, nhận: {err}"
        );
    }

    #[tokio::test]
    async fn rebind_to_metadata_ip_is_blocked_by_resolved_check() {
        let g = guard_with(
            &[],
            FakeResolver::single(
                "cloud.rebind.attacker.com",
                std::net::IpAddr::from([169, 254, 169, 254]),
            ),
        );
        let err = g
            .check_resolved("http://cloud.rebind.attacker.com/latest/meta-data")
            .await
            .unwrap_err();
        assert!(matches!(err, GuardError::BlockedInternalTarget(_)));
    }

    #[tokio::test]
    async fn public_resolved_ip_is_allowed() {
        let g = guard_with(
            &[],
            FakeResolver::single("example.com", std::net::IpAddr::from([93, 184, 216, 34])),
        );
        let ok = g.check_resolved("http://example.com/").await.unwrap();
        assert!(!ok.whitelisted);
    }

    #[tokio::test]
    async fn unresolved_host_is_allowed_and_whitelisted_skips_dns() {
        // Không phân giải được: không có IP nào để kiểm ⇒ cho đi (Chromium sẽ
        // tự báo lỗi DNS), đúng bảng quy tắc.
        let g = guard_with(&[], Arc::new(FakeResolver { table: vec![] }));
        let ok = g
            .check_resolved("http://never-resolves.example/")
            .await
            .unwrap();
        assert!(!ok.whitelisted);

        // Origin trong whitelist bỏ qua phân giải dù host trỏ nội bộ — môi
        // trường dev người dùng khai đích danh.
        let dev = guard_with(
            &["http://dev.internal:3000"],
            FakeResolver::single("dev.internal", std::net::IpAddr::from([10, 0, 0, 5])),
        );
        let ok = dev
            .check_resolved("http://dev.internal:3000/")
            .await
            .unwrap();
        assert!(ok.whitelisted);
    }

    // -----------------------------------------------------------------------
    // Ngoại lệ loopback: chỉ mở khi whitelist khai tên
    // -----------------------------------------------------------------------

    #[test]
    fn loopback_is_allowed_only_when_explicitly_whitelisted() {
        let g = guard(&["http://localhost:3000"]);
        let ok = g.check("http://localhost:3000/app").unwrap();
        assert!(ok.whitelisted);
        assert!(ok.origin.is_loopback());
    }

    #[test]
    fn loopback_ip_literal_is_blocked_without_whitelist() {
        // Cùng một đích, nhưng người dùng chưa khai `127.0.0.1` ⇒ chặn.
        let g = guard(&["http://localhost:3000"]);
        let err = g.check("http://127.0.0.1:3000/").unwrap_err();
        assert!(
            matches!(err, GuardError::BlockedInternalTarget(_)),
            "phải chặn, nhận: {err}"
        );
    }

    #[test]
    fn whitelist_covers_127_0_0_1_too_when_declared() {
        let g = guard(&["http://127.0.0.1:3000", "http://localhost:3000"]);
        assert!(g.check("http://127.0.0.1:3000/").unwrap().whitelisted);
        assert!(g.check("http://localhost:3000/").unwrap().whitelisted);
    }

    #[test]
    fn private_ranges_are_blocked_outside_whitelist() {
        let g = guard(&[]);
        for url in [
            "http://10.0.0.5/admin",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/", // metadata cloud
            "http://172.16.0.1/",
            "http://[::1]/",
            "http://[fd00::1]/",
        ] {
            let err = g.check(url).unwrap_err();
            assert!(
                matches!(err, GuardError::BlockedInternalTarget(_)),
                "{url} phải bị chặn, nhận: {err}"
            );
        }
    }

    #[test]
    fn public_urls_pass_but_are_not_whitelisted() {
        let g = guard(&[]);
        let v = g.check("https://example.com/docs").unwrap();
        assert!(
            !v.whitelisted,
            "truy cập internet được phép nhưng KHÔNG phải whitelist"
        );
    }

    // -----------------------------------------------------------------------
    // Ràng buộc định dạng
    // -----------------------------------------------------------------------

    #[test]
    fn non_http_schemes_are_rejected() {
        let g = guard(&[]);
        for url in [
            "file:///etc/passwd",
            "ftp://x.internal/",
            "javascript:alert(1)",
        ] {
            assert!(
                matches!(
                    g.check(url),
                    Err(GuardError::UnsupportedScheme(_)) | Err(GuardError::Malformed)
                ),
                "{url} phải bị từ chối"
            );
        }
    }

    #[test]
    fn credential_and_oversized_urls_are_rejected() {
        let g = guard(&[]);
        assert_eq!(
            g.check("https://user:pw@example.com/").unwrap_err(),
            GuardError::EmbeddedCredentials
        );
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_CHARS));
        assert_eq!(g.check(&long).unwrap_err(), GuardError::UrlTooLong);
    }

    #[test]
    fn empty_whitelist_blocks_all_internal_targets() {
        // Whitelist rỗng = "không tin ai", nên loopback cũng chặn — kể cả khi viết
        // bằng TÊN (`localhost`) chứ không phải IP literal. Đây là khe đã bị test bắt
        // và đã sửa: chỉ kiểm IP literal thì `localhost` lọt qua.
        let g = guard(&[]);
        for url in [
            "http://localhost:5173/",
            "http://127.0.0.1:5173/",
            "http://[::1]:5173/",
        ] {
            assert!(
                g.check(url).is_err(),
                "{url} phải bị chặn khi whitelist rỗng"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Tích hợp với so khớp wildcard: lớp guard dùng đúng verdict đó
    // -----------------------------------------------------------------------

    #[test]
    fn guard_uses_wildcard_matching_not_suffix_matching() {
        let g = guard(&["http://*.dev.internal:3000"]);
        assert!(
            g.check("http://api.dev.internal:3000/")
                .unwrap()
                .whitelisted
        );
        // Domain giả dạng: có hậu tố trùng nhưng KHÔNG nằm trong ranh giới label.
        assert!(
            !g.check("http://dev.internal.attacker.com:3000/")
                .unwrap()
                .whitelisted
        );
    }

    #[test]
    fn whitelist_verdict_matches_origin_module() {
        // Lớp guard KHÔNG tự chấn đoán: nó chỉ dùng lại `OriginWhitelist`, nên
        // không thể lệch quyết định với tầng risk.
        let wl = list(&["https://*.dev.internal"]);
        let g = BrowserGuard::new(Arc::new(wl.clone()));
        let v = g.check("https://api.dev.internal/x").unwrap();
        assert_eq!(
            OriginVerdict::Whitelisted,
            wl.verdict(&v.origin),
            "hai tầng phải cùng kết luận"
        );
    }
}
