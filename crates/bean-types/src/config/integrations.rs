//! Section `[[mcp_servers]]`, `[mcp]`, `[browser]`.

use std::collections::BTreeMap;

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

/// Một mục `[[mcp_servers]]` (agents.md mục 16).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    /// Tên server, dùng làm tiền tố `mcp__<server>__<tool>`.
    pub name: String,
    /// Lệnh khởi chạy (có thể là `docker run …` để bọc server không tin cậy).
    pub command: String,
    /// Tham số dòng lệnh.
    #[serde(default)]
    pub args: Vec<String>,
    /// Biến môi trường truyền cho tiến trình con.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// `true` ⇒ tool của server hạ xuống `Safe`; mặc định `false` ⇒ `Confirm`.
    #[serde(default)]
    pub trust: bool,
    /// Timeout cho **một lần gọi tool** của server này (giây).
    ///
    /// `None` ⇒ dùng mặc định 60 giây của [`McpTimeouts`](https://docs.rs/bean-tools).
    /// Cần cho tool chạy lâu thực sự (truy vấn SIEM, xuất báo cáo…), vì bị cắt ở 60s sẽ
    /// thành tool error *giả* khiến model tưởng server hỏng rồi thử lại vô ích.
    ///
    /// Giá trị phải nằm trong `1..=`[`MAX_MCP_CALL_TIMEOUT_SECONDS`].
    #[serde(default)]
    pub call_timeout_seconds: Option<u32>,
    /// Biến môi trường của tiến trình Bean được phép **kế thừa** cho server này.
    ///
    /// Mặc định rỗng, và đó là hành vi đúng: `connect_service` dùng `env_clear()` để
    /// server không thấy secret của host (mục 15.6). Nhưng `env_clear()` xoá luôn
    /// `PATH`, nên **MCP server cần `PATH` để chạy sẽ chết ngay** — ví dụ wrapper có
    /// shebang `#!/usr/bin/env <interpreter>`, thiếu `PATH` thì `env` không tìm thấy
    /// trình thông dịch và tiến trình con thoát với status 127.
    ///
    /// Trường này là cách thay đổi *có kiểm soát* thay vì bỏ `env_clear()`: chỉ những
    /// biến được liệt kê mới được truyền, và `env` tường minh luôn thắng.
    #[serde(default)]
    pub inherit_env: Vec<String>,
    /// Tag RBAC mà role phải giữ để thấy/cọp tool của server này (M22).
    ///
    /// Rỗng (mặc định) ⇒ mọi role đã được cấp quyền đều thấy, giữ hành vi cũ. Đặt
    /// `["infra-read"]` cho server SIEM/CVE để chỉ role giám sát mới thấy (xem
    /// `docs/decisions.md` D12.1).
    #[serde(default)]
    pub tool_tags: Vec<String>,
}

/// Một client MCP được phép gọi vào Bean (M25, `[[mcp_clients]]`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpClientConfig {
    /// Tên client (kebab-case), ví dụ `cline`, `cursor`, `opencode`.
    ///
    /// Danh tính đầy đủ là `mcp-client:<name>` — **tách biệt hoàn toàn** khỏi
    /// `telegram:<id>` / `web:admin` (Plan.md M25 mục 2): token của một client bị lộ
    /// không tự động đổi thành quyền của bạn qua Telegram.
    pub name: String,
    /// Role mà client này dùng; phải có trong `[[roles]]` và trong
    /// `agent.user_roles["mcp-client:<name>"]` (validate kiểm ở tầng code).
    ///
    /// Cấu hình này là **chính sách**, không phải credential: hash token nằm trong
    /// SQLite (`data.dir/bean.db`) nên `bean.toml` — thường được commit —
    /// không bao giờ chứa bí mật (xem `Store::create_mcp_client`).
    pub role: String,
}

/// `[mcp_server]` — Bean đóng vai **MCP server** read-only (M25).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerConfigSettings {
    /// Bật chế độ MCP server. Mặc định `false` (bề mặt tấn công mới, phải bật tường minh).
    pub enabled: bool,
    /// Bật transport HTTP (streamable-HTTP/SSE). stdio luôn khả dụng qua `bean mcp`.
    pub http_enabled: bool,
    /// Địa chỉ bind cho transport HTTP; mặc định chỉ loopback, giống `[web]` (mục 15.7).
    pub bind: SocketAddr,
    /// Origin công khai của endpoint MCP (kiểm `Host`/`Origin` khi bật HTTP).
    pub public_origin: String,
    /// Cho phép bind ngoài loopback — phải đặt sau reverse proxy TLS/Tailscale.
    pub allow_remote: bool,
    /// TTL của token client (giờ). `0` ⇒ không hết hạn.
    pub token_ttl_hours: u32,
    /// Cho phép truy cập theo `Host` khác `public_origin` (mặc định chỉ loopback).
    pub allowed_hosts: Vec<String>,
    /// Số request/phút tối đa cho **một token** trên transport HTTP (K24).
    ///
    /// Chỉ có tác dụng khi `http_enabled = true`. `0` = không giới hạn. Mặc định 120
    /// đủ rộng cho một phiên coding dài (agent gọi tool liên tục) nhưng vẫn chặn được
    /// token lộ bị dùng để quét dữ liệu.
    pub rate_limit_per_minute: u32,
    /// Hệ số nhân thêm cho giới hạn theo **IP** so với giới hạn theo token.
    ///
    /// IP phải rộng hơn vì nhiều client hợp lệ có thể đi chung một IP (reverse proxy,
    /// NAT, nhiều IDE trên cùng máy). `0` = tắt giới hạn theo IP.
    pub rate_limit_ip_multiplier: u32,
}

impl Default for McpServerConfigSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            http_enabled: false,
            bind: SocketAddr::from(([127, 0, 0, 1], 7879)),
            public_origin: "http://127.0.0.1:7879".to_string(),
            allow_remote: false,
            token_ttl_hours: 8760,
            allowed_hosts: Vec::new(),
            rate_limit_per_minute: 120,
            rate_limit_ip_multiplier: 5,
        }
    }
}

/// Cấu hình `[browser]` — tool browser nội bộ nói thẳng CDP (M26).
///
/// **Mặc định tắt hoàn toàn** (`enabled = false`): bản cài không có Chrome thì vẫn
/// chạy bình thường, và tool browser không xuất hiện trong payload gửi model.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserConfig {
    /// Bật nhóm tool browser.
    pub enabled: bool,
    /// Đường dẫn tới Chrome/Chromium **đã cài sẵn**.
    ///
    /// Rỗng ⇒ tự phát hiện theo thứ tự: biến môi trường `CHROME` → tên binary trong
    /// `PATH` (`google-chrome`, `chromium`…) → đường dẫn cố định kiểu
    /// `/opt/google/chrome`. Không tìm thấy thì tool báo lỗi rõ ràng.
    ///
    /// **Không bao giờ tải Chrome lúc chạy** — crate `chromiumoxide` được bật với
    /// `default-features = false` nên `chromiumoxide_fetcher` không tồn tại trong
    /// binary (nguyên tắc "cài trước, không tải lúc chạy", xem `docs/decisions.md` D26.2).
    pub executable_path: String,
    /// Origin được phép chạy tool **hành động** ở mức `Confirm` (có "cho phép trong
    /// phiên"). Origin ngoài danh sách này luôn là `Dangerous`.
    ///
    /// Chấp nhận `scheme://host[:port]` và `scheme://*.host[:port]`. Wildcard khớp
    /// **theo ranh giới label**: `*.dev.internal` khớp `api.dev.internal` nhưng
    /// **không** khớp `dev.internal.attacker.com`.
    ///
    /// **Rỗng ⇒ mọi origin ngoài whitelist đều `Dangerous`**; danh sách rỗng không
    /// bao giờ có nghĩa "cho phép tất cả". Đây cũng là nơi duy nhất mở ngoại lệ
    /// loopback (xem `bean-browser::guard`).
    pub allowed_origins: Vec<String>,
    /// Chạy Chrome không có cửa sổ (mặc định `true`).
    pub headless: bool,
    /// Idle bao lâu thì tự tắt Chrome để giải phóng bộ nhớ (giây). `0` ⇒ không tự
    /// tắt, chỉ tắt khi Bean dừng.
    pub idle_timeout_seconds: u32,
    /// Số giây chờ tối đa cho một lần khởi động Chrome.
    pub launch_timeout_seconds: u64,
    /// Kích thước ảnh tối đa cho một lần chụp (byte). Ảnh vượt ngưỡng bị từ chối
    /// kèm thông điệp rõ ràng thay vì âm thầm làm lịch sử phình.
    pub max_image_bytes: usize,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable_path: String::new(),
            allowed_origins: Vec::new(),
            headless: true,
            idle_timeout_seconds: 300,
            launch_timeout_seconds: 30,
            max_image_bytes: 4 * 1024 * 1024,
        }
    }
}
