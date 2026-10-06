//! Section `[web]` và `[telegram]`.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use super::*;

/// `[web]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebConfig {
    /// Bật giao diện web.
    pub enabled: bool,
    /// Địa chỉ bind; mặc định chỉ loopback (mục 15.7).
    pub bind: SocketAddr,
    /// Origin công khai mà UI và WebSocket phải khớp (chống CSRF/CSWSH).
    pub public_origin: String,
    /// Cho phép bind ra ngoài loopback (bật tường minh + đặt sau reverse proxy TLS).
    pub allow_remote: bool,
    /// TTL của phiên đăng nhập (giờ).
    pub session_ttl_hours: u32,
    /// Định danh (`user_id`) mà phiên đăng nhập web sẽ mang.
    ///
    /// # Vì sao phải cấu hình được
    ///
    /// Trước đây định danh này **ghim cứng** thành `web:admin` ở
    /// `bean-web`, nên mọi phiên web đều là một user duy nhất và RBAC ở tầng
    /// API không bao giờ có dữ liệu để lọc — `finance-readonly` không thể tồn tại
    /// trên kênh web, dù cấu hình đã khai báo role đó.
    ///
    /// Đặt được ở đây thì một bản triển khai có thể bán giao diện web cho một user
    /// có role riêng, và HUB chỉ trả về đúng domain mà user đó được phép thấy.
    ///
    /// Mặc định `web:admin` giữ nguyên hành vi một-người-dùng của v1. Giá trị này
    /// phải có trong `agent.allowed_users` và được map trong `agent.user_roles`,
    /// nếu không thì user đó là `no-access` (fail-closed theo D11.1) và HUB trống.
    #[serde(default = "default_web_user")]
    pub user_id: String,
    /// Tin `X-Forwarded-For`/`X-Real-IP` (chỉ khi chạy sau reverse proxy tin cậy — D4.3).
    pub trust_proxy: bool,
}

/// Định danh web mặc định; giữ hành vi một-người-dùng của v1 khi không khai báo.
fn default_web_user() -> String {
    DEFAULT_WEB_USER.to_string()
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: SocketAddr::from(([127, 0, 0, 1], 7878)),
            public_origin: "http://127.0.0.1:7878".to_string(),
            allow_remote: false,
            session_ttl_hours: 168,
            trust_proxy: false,
            user_id: default_web_user(),
        }
    }
}

/// `[telegram]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelegramConfig {
    /// Bật kênh Telegram.
    pub enabled: bool,
    /// Tên biến môi trường chứa bot token.
    pub token_env: String,
    /// Allowlist user id — **bắt buộc** khi `enabled = true` (mục 13).
    pub allowed_user_ids: Vec<i64>,
    /// Giới hạn số tin mỗi phút cho mỗi chat.
    pub rate_limit_per_minute: u32,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token_env: "TELEGRAM_BOT_TOKEN".to_string(),
            allowed_user_ids: Vec::new(),
            rate_limit_per_minute: 20,
        }
    }
}
