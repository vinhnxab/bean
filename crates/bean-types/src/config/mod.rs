//! Cấu hình `bean.toml` (agents.md mục 18).
//!
//! Nguyên tắc:
//! * **Không bao giờ** chứa secret trong file cấu hình — chỉ tên biến môi trường (`*_env`).
//!   Giá trị thật lấy qua [`Config::resolve_secrets_with`] và bọc `secrecy::SecretString`.
//! * `#[serde(deny_unknown_fields)]`: gõ sai khoá là lỗi ngay, không im lặng bỏ qua.
//! * Mọi trường đều có giá trị mặc định để `chat` chạy được khi chưa có `bean.toml`
//!   (xem `docs/decisions.md` D5.11), nhưng `validate()` vẫn bắt các tổ hợp vô nghĩa.
//! ## Bản đồ module
//!
//! ```text
//! config/
//! ├─ mod.rs                   Config, ResolvedSecrets, hằng số, re-export
//! ├─ error.rs                 ConfigError, expand_tilde
//! ├─ enums.rs                 enum dùng chung
//! ├─ agent.rs                 [agent] [roles] [projects] [data]
//! ├─ llm.rs                   [llm] [tools.web_search] [learning]
//! ├─ tools.rs                 [tools] [security.sandbox] [security]
//! ├─ channels.rs              [web] [telegram]
//! ├─ integrations.rs          [[mcp_servers]] [mcp] [browser]
//! ├─ products.rs              [billing] [scan] [marketing] [qa]
//! ├─ load.rs                  load / load_or_default / validate
//! ├─ access.rs                accessor rbac / role / mcp / project
//! ├─ validate.rs              validate lõi
//! ├─ validate_integrations.rs validate tích hợp
//! └─ secrets.rs               resolve_* từ biến môi trường
//! ```
//!
//! Test nằm ở `tests/config.rs`.

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::fmt;

mod access;
mod agent;
mod channels;
mod enums;
mod error;
mod integrations;
mod llm;
mod load;
mod products;
mod secrets;
mod tools;
mod validate;
mod validate_integrations;

pub use agent::{AgentConfig, DataConfig, ProjectConfig, RoleConfig};
pub use channels::{TelegramConfig, WebConfig};
pub use enums::{LlmProviderKind, QaRunner, SandboxMode, ScanTargetKind, WebSearchProvider};
pub use error::ConfigError;
pub use integrations::{BrowserConfig, McpClientConfig, McpServerConfig, McpServerConfigSettings};
pub use llm::{LearningConfig, LlmConfig, WebSearchConfig};
pub use products::{
    BillingConfig, MarketingConfig, QaConfig, QaSandboxConfig, QaSuiteConfig, ScanSandboxConfig,
    ScanScopeEntry, SecurityScanConfig,
};
pub use tools::{SandboxConfig, SecurityConfig, ToolsConfig};
pub use validate::validate_scope_value;

/// Tên file cấu hình mặc định ở gốc workspace.
pub const DEFAULT_CONFIG_FILE: &str = "bean.toml";

/// Định danh `user_id` mặc định của kênh web.
///
/// Khai báo ở `bean-types` (không phải `bean-web`) vì `Config` cần giá trị
/// này làm **mặc định serde** mà không được phụ thuộc vào crate web — hướng phụ thuộc
/// của workspace là `types ← tools ← core ← web`.
pub const DEFAULT_WEB_USER: &str = "web:admin";

/// Tên nhóm tool browser trong `[tools] enabled` (M26).
pub const BROWSER_TOOL_GROUP: &str = "browser";

/// Tên nhóm tool QA trong `[tools] enabled` (M27).
pub const QA_TOOL_GROUP: &str = "qa";

/// Nhóm tool hợp lệ cho `[tools] enabled` (agents.md mục 7.3).
pub const KNOWN_TOOL_GROUPS: &[&str] = &[
    "files",
    "shell",
    "web",
    "memory",
    "skills",
    "schedule",
    // Nhóm tool browser nội bộ (M26) — nói thẳng CDP, không qua MCP/npm. Vẫn cần
    // `[browser].enabled = true` mới thực sự đăng ký (xem `validate_browser`).
    BROWSER_TOOL_GROUP,
    BILLING_TOOL_GROUP,
    QA_TOOL_GROUP,
];

/// Tên nhóm tool billing trong `[tools] enabled`.
pub const BILLING_TOOL_GROUP: &str = "billing";

/// Tag RBAC của domain tài chính (M22a).
pub const BILLING_TAG: &str = "billing-read";

// Tag RBAC của domain marketing (M24) định nghĩa ở `rbac.rs`; re-export ở đây để các
// tool import cùng chỗ với `INFRA_SCAN_TAG`/`BILLING_TAG`.
pub use crate::rbac::{MARKETING_DRAFT_TAG, MARKETING_PUBLISH_TAG, MARKETING_READ_TAG};

/// Tag RBAC của domain quét bảo mật (M23).
///
/// Cùng giá trị với tag mà `run_shell` mang, để vai trò trực trật bảo mật chạy được cả tool
/// quét lẫn `run_shell` — nguyên lý four-eyes: vai trò này cần cả hai.
pub const INFRA_SCAN_TAG: &str = "infra-scan";

/// Tag RBAC chỉ đọc hạ tầng (M22) — đọc log/CVE/uptime qua MCP client.
pub const INFRA_READ_TAG: &str = "infra-read";

/// Tag RBAC của tool `memory_query` (M25) — đọc `MEMORY.md`/`USER.md`, **không** cho sửa.
///
/// Tag riêng, không dùng lại `memory` group: M25 yêu cầu đường MCP server chỉ expose
/// đúng ba tag `infra-read`/`billing-read`/`memory-read`, nên danh tính "đọc bộ nhớ"
/// phải là một tag tường minh chứ không phải "tool không gắn tag" (xem D16.1).
pub const MEMORY_READ_TAG: &str = "memory-read";

/// Tag RBAC cho phép **ghi** code (M21.2) — vai trò `developer`.
///
/// Tách riêng khỏi `infra-scan` để nguyên tắc four-eyes suy ra được: role review
/// (`qa`) giữ `dev-read`/`test-run` nhưng tuyệt đối không có tag này, và
/// `validate_rbac` chặn trường hợp vừa cấp vừa cấm.
pub const DEV_WRITE_TAG: &str = "dev-write";

/// Tag RBAC cho phép **đọc** code để review — không kèm quyền ghi.
pub const DEV_READ_TAG: &str = "dev-read";

/// Tag RBAC cho phép chạy bộ test.
pub const TEST_RUN_TAG: &str = "test-run";

/// Tên project mặc định — ánh xạ tới `agent.workspace` (M21.1).
pub const DEFAULT_PROJECT: &str = "default";

/// Trần cho [`McpServerConfig::call_timeout_seconds`] (giây).
///
/// Một giờ là trần có chủ đích: tool chạy lâu thực sự (truy vấn SIEM, xuất báo cáo, quét
/// hạ tầng…) cần hơn 60 giây, nhưng một MCP server treo vô hạn sẽ giữ cả slot của agent
/// loop (`max_steps`) — lỗi thật sự nằm ở chỗ không có trần.
pub const MAX_MCP_CALL_TIMEOUT_SECONDS: u32 = 3600;

/// Tiền tố danh tính của client MCP (M25) — **tách biệt hoàn toàn** khỏi `telegram:`/`web:`.
///
/// `Router::authorize` yêu cầu `user_id` bắt đầu bằng `<channel>:`; MCP client đi qua
/// cổng riêng nên identity này được dựng từ `[[mcp_clients]].name`.
pub const MCP_CLIENT_PREFIX: &str = "mcp-client:";

/// Cấu hình gốc của Bean (`bean.toml`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// `[agent]`.
    pub agent: AgentConfig,
    /// `[data]`.
    pub data: DataConfig,
    /// `[llm]`.
    pub llm: LlmConfig,
    /// `[tools]`.
    pub tools: ToolsConfig,
    /// `[[roles]]` — bảng role của RBAC (M21.2).
    pub roles: Vec<RoleConfig>,
    /// `[billing]` — domain tài chính read-only (M22a).
    pub billing: BillingConfig,
    /// `[[infra_scope]]` — danh sách target **được phép quét** (M23).
    ///
    /// Rỗng ⇒ mọi lần quét đều bị từ chối (fail-closed). Đây là mặc định an toàn: agent
    /// không thể trở thành công cụ quét bất kỳ host nào khi chưa được khai báo rõ ràng.
    pub infra_scope: Vec<ScanScopeEntry>,
    /// `[security_scan]` — domain quét bảo mật (M23).
    pub security_scan: SecurityScanConfig,
    /// `[browser]` — tool browser nội bộ, nói thẳng CDP (M26).
    pub browser: BrowserConfig,
    /// `[marketing]` — domain marketing (M24).
    pub marketing: MarketingConfig,
    /// `[qa]` — domain chạy test suite cho vai trò `qa` (M27).
    pub qa: QaConfig,
    /// `[[projects]]` — project profile (M21.1). Rỗng ⇒ chỉ có project `default`
    /// ánh xạ tới `agent.workspace`.
    pub projects: Vec<ProjectConfig>,
    /// `[learning]` — reflection và duyệt skill nháp.
    pub learning: LearningConfig,
    /// `[security]`.
    pub security: SecurityConfig,
    /// `[web]`.
    pub web: WebConfig,
    /// `[telegram]`.
    pub telegram: TelegramConfig,
    /// `[[mcp_servers]]`.
    pub mcp_servers: Vec<McpServerConfig>,
    /// `[[mcp_clients]]` — client MCP nào được phép gọi vào Bean (M25).
    pub mcp_clients: Vec<McpClientConfig>,
    /// `[mcp_server]` — Bean đóng vai MCP server read-only (M25).
    pub mcp_server: McpServerConfigSettings,
}

/// Secret đã đọc từ biến môi trường.
///
/// **Không bao giờ** in giá trị: `Debug` chỉ cho biết biến nào đã được đặt (agents.md mục 15.6).
#[derive(Clone)]
pub struct ResolvedSecrets {
    /// API key của LLM provider.
    pub llm_api_key: Option<SecretString>,
    /// API key của `web_search` (bỏ trống khi provider là SearXNG hoặc nhóm `web` bị tắt).
    pub web_search_api_key: Option<SecretString>,
    /// Bot token Telegram (chỉ khi `telegram.enabled = true`).
    pub telegram_token: Option<SecretString>,
}

impl ResolvedSecrets {
    /// Đã có API key cho LLM chưa (dùng để báo lỗi sớm thay vì lộn xộn lúc gọi API).
    #[must_use]
    pub fn has_llm_api_key(&self) -> bool {
        self.llm_api_key.is_some()
    }
}

fn presence(secret: &Option<SecretString>) -> &'static str {
    if secret.is_some() { "<set>" } else { "<unset>" }
}

impl fmt::Debug for ResolvedSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedSecrets")
            .field("llm_api_key", &presence(&self.llm_api_key))
            .field("web_search_api_key", &presence(&self.web_search_api_key))
            .field("telegram_token", &presence(&self.telegram_token))
            .finish()
    }
}
