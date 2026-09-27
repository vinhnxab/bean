//! Giới hạn tần suất cho transport HTTP của MCP server (K24).
//!
//! # Vì sao chỉ HTTP, không phải stdio
//!
//! `docs/known-issues.md` K24 đã chốt: *"stdio thì token nằm trong tiến trình của chính
//! người dùng nên rủi ro thấp hơn nhiều"*. Thêm trần tần suất cho stdio sẽ **vỡ** phiên
//! làm việc dài mà không chặn được thêm kẻ tấn công nào — vì ở stdio, kẻ tấn công phải
//! đã nằm trong máy bạn. Vì vậy [`McpRateLimiter`] **chỉ** được gọi qua
//! `ServeContext::authenticate_http`; đường stdio gọi `authenticate_stdio` và không có
//! tham số limiter nào để truyền — không thể vô tình áp nhầm.
//!
//! # Ba lớp khoá, dùng chung một thuật toán
//!
//! | Lớp | Khoá theo | Ngưỡng | Chặn cái gì |
//! |---|---|---|---|
//! | thất bại | token + IP | 5 lần/60s, khoá tăng dần tới 300s | dò/brute-force token |
//! | lưu lượng | token | `rate_limit_per_minute` (120) | token lô bị dùng quá tần suất |
//! | lưu lượng | IP | ×`rate_limit_ip_multiplier` (600) | một IP điều khiển nhiều token |
//!
//! Thuật toán khoá là [`RateLimiter`] — **cùng** cái `POST /api/auth/login` đang dùng;
//! ở đây không viết thuật toán thứ hai.
//!
//! # Vì sao khoá theo `hash_token` chứ không theo token thô
//!
//! Token thô là bí mật dài hạn; đưa nó làm khoá trong `HashMap` sẽ giữ bí mật sống trong
//! bộ nhớ tiến trình (và có thể lọt vào core dump). [`hash_token`] cho ra khoá ổn định,
//! không đảo ngược được, mà vẫn tách được từng client.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use beanagent_security::ratelimit::{
    DEFAULT_MAX_LOCK, DEFAULT_THRESHOLD, DEFAULT_WINDOW, RateLimiter,
};
use beanagent_types::config::McpServerConfigSettings;

use crate::mcp_server::auth::hash_token;

/// Hệ số nhân mặc định cho trần theo IP so với trần theo token.
pub const DEFAULT_IP_MULTIPLIER: u32 = 5;

/// Kết quả một lần kiểm tra giới hạn tần suất.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitVerdict {
    /// Được phép đi tiếp.
    Allowed,
    /// Bị từ chối; `retry_after` là số giây client nên chờ.
    Limited {
        /// Còn phải chờ bao lâu (>= 1 giây).
        retry_after: Duration,
        /// Khoá nào đã vượt ngưỡng, để ghi log (không chứa bí mật).
        scope: &'static str,
    },
}

impl LimitVerdict {
    /// Cho phép hay không.
    #[must_use]
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Bộ giới hạn tần suất của transport HTTP, dựng từ cấu hình `[mcp_server]`.
///
/// `Clone` rẻ (chỉ có `Arc` bên trong) để chia sẻ giữa các request mà không phải chia
/// sẻ tham chiếu `&mut`.
#[derive(Debug, Clone)]
pub struct McpRateLimiter {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Dò token: khoá theo **cả** dấu vân tay token lẫn IP.
    auth_failures: RateLimiter,
    /// Lưu lượng hợp lệ theo token.
    per_token: RateLimiter,
    /// Lưu lượng hợp lệ theo IP.
    per_ip: RateLimiter,
}

impl McpRateLimiter {
    /// Dựng từ cấu hình. Trần `0` nghĩa là tắt lớp lưu lượng tương ứng (lớp chống dò
    /// token luôn bật — nó không phải thứ tuỳ chọn).
    #[must_use]
    pub fn new(settings: &McpServerConfigSettings) -> Self {
        let per_minute = settings.rate_limit_per_minute;
        let ip_limit = per_minute.saturating_mul(settings.rate_limit_ip_multiplier);
        Self {
            inner: Arc::new(Inner {
                // Lớp chống dò token: **đúng** ngưỡng của `POST /api/auth/login`.
                auth_failures: RateLimiter::with_limits(
                    DEFAULT_THRESHOLD,
                    DEFAULT_WINDOW,
                    DEFAULT_MAX_LOCK,
                ),
                per_token: volume_limiter(per_minute),
                per_ip: volume_limiter(ip_limit),
            }),
        }
    }

    /// Bộ giới hạn **không** giới hạn lưu lượng — dùng khi `rate_limit_per_minute = 0`.
    ///
    /// Lớp chống dò token vẫn giữ nguyên ngưỡng login: tắt trần lưu lượng là lựa chọn
    /// của người dùng, không phải lý do để mở lại đường dò token.
    #[must_use]
    pub fn unlimited() -> Self {
        Self::new(&McpServerConfigSettings {
            rate_limit_per_minute: 0,
            rate_limit_ip_multiplier: 0,
            ..McpServerConfigSettings::default()
        })
    }

    /// Kiểm tra trước khi xác thực. Phải chạy **trước** khi tra DB, nếu không kẻ tấn công
    /// dùng được endpoint này để bắn hàng loạt truy vấn `mcp_clients`.
    ///
    /// `token` là token **thô** nhận từ header; hàm tự băm, không giữ token thô.
    #[must_use]
    pub fn check(&self, token: Option<&str>, ip: IpAddr) -> LimitVerdict {
        let ip_key = ip_key(ip);
        match token.map(hash_token) {
            // Có token: khoá được **cả** theo token lẫn theo IP.
            Some(fingerprint) => {
                let key = token_key(&fingerprint);
                if let Err(retry_after) = self.inner.auth_failures.check(&key) {
                    return limited(retry_after, "token_failures");
                }
                if let Err(retry_after) = self.inner.auth_failures.check(&ip_key) {
                    return limited(retry_after, "ip_failures");
                }
                if let Err(retry_after) = self.inner.per_token.check(&key) {
                    return limited(retry_after, "token_volume");
                }
            }
            // Không có token: chỉ còn khoá theo IP để tính.
            None => {
                if let Err(retry_after) = self.inner.auth_failures.check(&ip_key) {
                    return limited(retry_after, "ip_failures");
                }
            }
        }
        if let Err(retry_after) = self.inner.per_ip.check(&ip_key) {
            return limited(retry_after, "ip_volume");
        }
        LimitVerdict::Allowed
    }

    /// Ghi nhận **một lần xác thực sai**. Chỉ gọi khi token thật sự không hợp lệ.
    pub fn record_auth_failure(&self, token: Option<&str>, ip: IpAddr) {
        if let Some(fingerprint) = token.map(hash_token) {
            self.inner
                .auth_failures
                .record_failure(&token_key(&fingerprint));
        }
        self.inner.auth_failures.record_failure(&ip_key(ip));
    }

    /// Ghi nhận **một request hợp lệ** đã đi qua (để tính trần lưu lượng).
    pub fn record_request(&self, token: &str, ip: IpAddr) {
        self.inner
            .per_token
            .record_request(&token_key(&hash_token(token)));
        self.inner.per_ip.record_request(&ip_key(ip));
    }

    /// Xác thực thành công ⇒ xoá bộ đếm thất bại của token đó (không phải của IP: IP
    /// khoá cho mọi client đi qua nó, nên phải tự hết cửa sổ).
    pub fn record_auth_success(&self, token: &str) {
        self.inner
            .auth_failures
            .clear(&token_key(&hash_token(token)));
    }
}

/// Trần lưu lượng dùng **cùng** thuật toán khoá của lớp chống dò token: vượt ngưỡng
/// trong cửa sổ 60s thì khoá, cửa sổ mới mở lại. `0` ⇒ `u32::MAX` ⇒ không bao giờ vượt.
fn volume_limiter(per_minute: u32) -> RateLimiter {
    RateLimiter::with_limits(
        if per_minute == 0 {
            u32::MAX
        } else {
            per_minute
        },
        DEFAULT_WINDOW,
        DEFAULT_MAX_LOCK,
    )
}

fn limited(retry_after: Duration, scope: &'static str) -> LimitVerdict {
    LimitVerdict::Limited {
        // Luôn >= 1 giây: client cần một khoảng thời gian rõ ràng để thử lại, và `0`
        // sẽ khiến client dồn request ngay lập tức.
        retry_after: retry_after.max(Duration::from_secs(1)),
        scope,
    }
}

/// Khoá theo IP — cùng định dạng với `AuthService` của web.
fn ip_key(ip: IpAddr) -> String {
    format!("ip:{ip}")
}

/// Khoá theo dấu vân tay token (SHA-256 hex), **không** phải token thô.
fn token_key(fingerprint: &str) -> String {
    format!("token:{fingerprint}")
}
