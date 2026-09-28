//! Cấu hình `BeanAgent.toml` (agents.md mục 18).
//!
//! Nguyên tắc:
//! * **Không bao giờ** chứa secret trong file cấu hình — chỉ tên biến môi trường (`*_env`).
//!   Giá trị thật lấy qua [`Config::resolve_secrets_with`] và bọc `secrecy::SecretString`.
//! * `#[serde(deny_unknown_fields)]`: gõ sai khoá là lỗi ngay, không im lặng bỏ qua.
//! * Mọi trường đều có giá trị mặc định để `chat` chạy được khi chưa có `BeanAgent.toml`
//!   (xem `docs/decisions.md` D5.11), nhưng `validate()` vẫn bắt các tổ hợp vô nghĩa.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::rbac::{NO_ACCESS_ROLE, RolePermissions, WILDCARD_TAG};
use crate::router::AgentRelation;

/// Tên file cấu hình mặc định ở gốc workspace.
pub const DEFAULT_CONFIG_FILE: &str = "BeanAgent.toml";

/// Định danh `user_id` mặc định của kênh web.
///
/// Khai báo ở `beanagent-types` (không phải `beanagent-web`) vì `Config` cần giá trị
/// này làm **mặc định serde** mà không được phụ thuộc vào crate web — hướng phụ thuộc
/// của workspace là `types ← tools ← core ← web`.
pub const DEFAULT_WEB_USER: &str = "web:admin";

/// Tên nhóm tool browser trong `[tools] enabled` (M26).
pub const BROWSER_TOOL_GROUP: &str = "browser";

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
];

/// Lỗi khi nạp/kiểm tra cấu hình.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Không đọc được file.
    #[error("không đọc được file cấu hình `{path}`: {source}")]
    Read {
        /// Đường dẫn đã thử đọc.
        path: PathBuf,
        /// Lỗi I/O gốc.
        #[source]
        source: std::io::Error,
    },
    /// TOML sai cú pháp hoặc sai kiểu.
    #[error("file cấu hình `{path}` không hợp lệ: {source}")]
    Parse {
        /// Đường dẫn file.
        path: PathBuf,
        /// Lỗi TOML gốc (có số dòng/cột).
        #[source]
        source: toml::de::Error,
    },
    /// Cấu hình đọc được nhưng vô nghĩa/không an toàn.
    #[error("cấu hình sai: {0}")]
    Invalid(String),
    /// Thiếu biến môi trường cho secret.
    #[error("thiếu biến môi trường `{env}` (được khai báo bởi trường `{field}`)")]
    MissingEnv {
        /// Trường trong `BeanAgent.toml` khai báo biến này.
        field: &'static str,
        /// Tên biến môi trường.
        env: String,
    },
}

/// Provider LLM (`[llm] provider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    /// Anthropic Messages API.
    Anthropic,
    /// Mọi endpoint tương thích OpenAI Chat Completions (OpenAI, Ollama, vLLM…).
    // `snake_case` của variant này là `open_ai_compat` — đặt tên tường minh để khớp
    // `as_str()` và `BeanAgent.example.toml`.
    #[serde(rename = "openai_compat")]
    OpenAiCompat,
}

impl LlmProviderKind {
    /// Chuỗi như trong file cấu hình.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAiCompat => "openai_compat",
        }
    }
}

/// Chế độ sandbox cho `run_shell` (agents.md mục 15.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Mặc định: `docker run --rm` với workspace mount, không mạng, non-root.
    Docker,
    /// Chạy thẳng trên host — phải bật tường minh, mọi lệnh là `Dangerous`.
    Host,
}

impl SandboxMode {
    /// Chuỗi như trong file cấu hình.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Host => "host",
        }
    }
}

/// Provider cho `web_search` (agents.md mục 7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchProvider {
    /// Tavily.
    Tavily,
    /// Brave Search.
    Brave,
    /// SearXNG tự host (không cần API key).
    Searxng,
}

impl WebSearchProvider {
    /// Provider này có cần API key không?
    #[must_use]
    pub const fn requires_api_key(self) -> bool {
        !matches!(self, Self::Searxng)
    }
}

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

/// `[billing]` — domain tài chính, read-only (M22a).
///
/// Tách biệt hoàn toàn khỏi domain `infra-*`: tool ở đây chỉ **đọc chi phí**, không đụng
/// tới hạ tầng.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BillingConfig {
    /// Bật tool đọc chi phí. Mặc định `false`.
    pub enabled: bool,
    /// Tên biến môi trường chứa credential billing.
    ///
    /// **Phải là biến riêng** — `validate()` từ chối nếu trùng với biến của LLM, web search
    /// hay Telegram, vì credential có quyền quản trị hạ tầng không được dùng lại ở đây
    /// (`Plan.md` mục 18, D13.2).
    pub api_key_env: String,
    /// Endpoint trả JSON chi phí. `None` ⇒ tool chạy ở **chế độ stub**, không gọi mạng.
    pub base_url: Option<String>,
    /// Gắn thêm tham số truy vấn vào `base_url` khi gọi (ví dụ `?period=30d`).
    pub query_suffix: Option<String>,
}

impl Default for BillingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_env: "CLOUD_BILLING_API_KEY".to_string(),
            base_url: None,
            query_suffix: None,
        }
    }
}

/// Kiểu target trong `[[infra_scope]]` (M23).
///
/// **Chỉ chấp nhận `ip`/`cidr`**, cố ý **không** có `hostname`: nếu cho phép tên miền thì
/// code kiểm scope và scanner sẽ phân giải DNS ở hai thời điểm khác nhau, tạo khe hở TOCTOU
/// (check ra IP được phép, lúc chạy lại ra IP khác) — xem `docs/decisions.md` D14.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanTargetKind {
    /// Một địa chỉ IP đơn lẻ, ví dụ `203.0.113.7`.
    Ip,
    /// Một dải CIDR, ví dụ `192.168.10.0/24`.
    Cidr,
}

/// Một mục `[[infra_scope]]` — target được phép quét (M23).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScanScopeEntry {
    /// Kiểu target.
    pub kind: ScanTargetKind,
    /// Giá trị: IP hoặc CIDR. Cú pháp được `validate()` kiểm tra nghiêm ngặt.
    pub value: String,
    /// Nhãn mô tả để người đọc báo cáo hiểu đang quét cái gì (tùy chọn).
    #[serde(default)]
    pub label: String,
}

/// Sandbox riêng cho tool quét (M23).
///
/// **Tách khỏi** `[security.sandbox]` của `run_shell` một cách tuyệt đối: scanner **bắt buộc**
/// phải có mạng để tới target, trong khi `run_shell` phải `--network none`. Dùng chung cấu
/// hình sẽ vô hiệu hoá cách ly của `run_shell` (D14.3).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanSandboxConfig {
    /// `docker` (mặc định, an toàn) hoặc `host` (chạy thẳng trên máy — phải bật tường minh).
    pub mode: SandboxMode,
    /// Image chứa scanner (chỉ dùng khi `mode = "docker"`).
    pub image: String,
    /// Cho container scanner ra mạng. **Luôn phải là `true`** — không có mạng thì không quét
    /// được target trong scope; `validate()` từ chối `false`.
    pub network: bool,
    /// Trần bộ nhớ.
    pub memory: String,
    /// Số CPU.
    pub cpus: f32,
    /// Trần số tiến trình trong container (chống fork bomb).
    pub pids_limit: u32,
    /// Thời gian tối đa cho một lần quét (giây).
    pub timeout_seconds: u64,
    /// Cho phép chạy scanner thẳng trên máy chủ. Mặc định `false`.
    ///
    /// `mode = "host"` mà quên bật cờ này sẽ bị `validate()` từ chối — chốt chặn chống vô
    /// tình hạ cấp cách ly (D14.4).
    #[serde(default)]
    pub allow_host: bool,
}

impl Default for ScanSandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::Docker,
            image: "instrumentisto/nmap:latest".to_string(),
            network: true,
            memory: "512m".to_string(),
            cpus: 1.0,
            pids_limit: 64,
            timeout_seconds: 300,
            allow_host: false,
        }
    }
}

impl ScanSandboxConfig {
    /// Thành [`SandboxConfig`] để dựng `Sandbox`, giữ nguyên mọi giới hạn an toàn sẵn có
    /// (non-root, `--cap-drop ALL`, no-new-privileges, trần bộ nhớ/CPU/pids).
    ///
    /// `SandboxConfig` không mang workspace — workspace truyền riêng khi dựng `Sandbox`.
    #[must_use]
    pub fn to_sandbox_config(&self) -> SandboxConfig {
        SandboxConfig {
            mode: self.mode,
            image: self.image.clone(),
            network: self.network,
            memory: self.memory.clone(),
            cpus: self.cpus,
            pids_limit: self.pids_limit,
            timeout_seconds: self.timeout_seconds,
        }
    }
}

/// Cấu hình domain marketing (M24).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarketingConfig {
    /// Bật nhóm tool marketing.
    pub enabled: bool,
    /// Tên biến môi trường chứa API key **riêng** của nền tảng publish.
    ///
    /// `validate()` từ chối trùng với `llm.api_key_env` / `tools.web_search.api_key_env` /
    /// `telegram.token_env` — cùng mẫu với M22a: quyền "chỉ post" phải là credential riêng,
    /// không dùng lại key có quyền rộng hơn.
    pub api_key_env: String,
    /// Endpoint nhận nội dung để đăng (JSON). Rỗng ⇒ **chế độ stub**: không gọi mạng.
    pub base_url: Option<String>,
    /// Trường JSON chứa nội dung trong body, ví dụ `"text"`.
    pub text_field: String,
}

impl Default for MarketingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_env: "MARKETING_POST_ONLY_KEY".to_string(),
            base_url: None,
            // `text` là tên trường phổ biến nhất; vẫn cấu hình được nếu provider khác.
            text_field: "text".to_string(),
        }
    }
}

/// Cấu hình domain `security-scan` (M23).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityScanConfig {
    /// Bật nhóm tool quét.
    pub enabled: bool,
    /// Sandbox riêng cho scanner.
    pub sandbox: ScanSandboxConfig,
    /// Kênh nhận cảnh báo mức cao (thường là kênh chính của bạn).
    ///
    /// Rỗng ⇒ không gửi cảnh báo trực tiếp. `Plan.md` yêu cầu cảnh báo mức cao gửi **thẳng**
    /// cho bạn, song song với báo cáo chuẩn hoá gửi Manager — không chỉ đi qua Manager lọc.
    pub alert_channel: String,
    /// `chat_id` của kênh cảnh báo.
    pub alert_chat_id: String,
}

/// `[agent]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// Thư mục làm việc của agent (mọi thao tác file bị jail trong đây).
    pub workspace: PathBuf,
    /// Tên agent hiển thị trong prompt và log.
    pub agent_name: String,
    /// Trần số bước của một run (agents.md mục 6).
    pub max_steps: u32,
    /// Ngân sách token cho context gửi model (mục 8.2, 8.3).
    pub context_budget_tokens: u32,
    /// Múi giờ IANA dùng để parse cron và hiển thị phía server (mục 14, D5.8).
    pub timezone: String,
    /// Lớp kiểm tra thứ hai ở lõi, ngoài allowlist của từng kênh (mục 10).
    pub allowed_users: Vec<String>,
    /// Map `user_id` (`telegram:<id>`, `web:admin`, `cli:local`) → tên role (M21.3).
    ///
    /// User **không** có trong map này dùng role mặc định
    /// [`NO_ACCESS_ROLE`](crate::NO_ACCESS_ROLE) — deny-all, an toàn theo mặc định.
    ///
    /// RBAC chỉ **bật** khi map này khác rỗng; để trống thì mọi user trong `allowed_users`
    /// giữ hành vi cũ (thấy mọi tool) để không phá cài đặt một-người-dùng (D10.3).
    pub user_roles: BTreeMap<String, String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            workspace: PathBuf::from("./workspace"),
            agent_name: "BeanAgent".to_string(),
            max_steps: 25,
            context_budget_tokens: 100_000,
            timezone: "Asia/Ho_Chi_Minh".to_string(),
            allowed_users: vec!["web:admin".to_string(), "cli:local".to_string()],
            user_roles: BTreeMap::new(),
        }
    }
}

/// Một role trong bảng `[[roles]]` (M21.2).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoleConfig {
    /// Tên role, khớp giá trị trong `agent.user_roles`.
    pub name: String,
    /// Các tag mà role được cấp. Tag `"*"` nghĩa là **mọi** quyền.
    #[serde(default)]
    pub tool_tags: Vec<String>,
    /// Tag **cấm** cứng cho role này; config sai thì `validate()` báo lỗi (M21.6).
    ///
    /// Dùng cho nguyên tắc four-eyes: role `qa` khai báo
    /// `forbid_tags = ["dev-write"]` nên **không thể** lỡ tay cấp quyền ghi code cho role
    /// review — kiểm tra ở tầng code, không dựa vào model tự kiểm tra.
    #[serde(default)]
    pub forbid_tags: Vec<String>,
    /// **Danh sách trắng tag**: role này chỉ thấy tool mang ít nhất một tag trong đây.
    ///
    /// Mặc định **rỗng ⇒ không áp dụng** (mọi role đã cấp quyền đều thấy tool untagged) —
    /// nên thêm trường này không đổi hành vi của cấu hình cũ nào.
    ///
    /// Cơ chế này sinh ra ở M24 để **tách domain**: role `marketing` phải thấy
    /// `web_fetch`/`web_search` mà **không** thấy `write_file`/`run_shell`/tool `infra-*`.
    /// Không thể làm bằng `tool_tags` vì gắn tag vào một tool untagged sẽ *giấu nó khỏi mọi
    /// role khác* — hồi quy cho các cài đặt đang chạy (D15.1).
    #[serde(default)]
    pub allowed_tool_tags: Vec<String>,
    /// Ghi đè `agent.context_budget_tokens` cho riêng role này (M21.7).
    #[serde(default)]
    pub context_budget_tokens: Option<u32>,
    /// Ghi đè `security.daily_token_budget` cho riêng role này (M21.7).
    #[serde(default)]
    pub daily_token_budget: Option<u64>,
}

impl RoleConfig {
    /// Tập tag đã chuẩn hoá (loại khoảng trắng thừa, bỏ tag rỗng).
    #[must_use]
    pub fn tag_set(&self) -> BTreeSet<String> {
        self.tool_tags
            .iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect()
    }

    /// Tập `allowed_tool_tags` đã chuẩn hoá (danh sách trắng tách domain — M24).
    #[must_use]
    pub fn allowed_tag_set(&self) -> BTreeSet<String> {
        self.allowed_tool_tags
            .iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect()
    }

    /// Quan hệ kiến trúc của role này với Manager, **suy ra từ tag được cấp**.
    ///
    /// # Vì sao suy ra từ tag chứ không khớp tên role
    ///
    /// Tên role (`developer`, `qa`, …) là lựa chọn của người cấu hình; cấu hình
    /// hoàn toàn hợp lệ có thể đặt tên khác (`dev`, `reviewer`). Nếu UI hardcode
    /// theo tên, một bản triển khai đổi tên sẽ **vẽ sai kiến trúc** mà không có
    /// ai báo lỗi — kiểu hỏng âm thầm nguy hiểm nhất.
    ///
    /// Tag thì ngược lại: nó là hợp đồng RBAC đã được kiểm ở tầng code
    /// (`validate_rbac`), và sửa đổi tên role không đổi được hành vi. Suy ra từ
    /// tag nên sơ đồ luôn mô tả đúng hệ thống đang chạy.
    ///
    /// Thứ tự ưu tiên có chủ đích:
    ///
    /// 1. `infra-scan` → [`AgentRelation::AlertsDirectly`]. Đây là kênh D14.11:
    ///    cảnh báo mức cao đi thẳng tới người quản trị, **không qua Manager**.
    ///    Kiểm trước vì vai trò trực trật thường *cũng* đọc hạ tầng.
    /// 2. Có tag đọc/kiểm thử nhưng **không** có tag ghi → [`AgentRelation::Reviews`].
    ///    Đây chính là định nghĩa four-eyes ở tầng dữ liệu: agent review được
    ///    đọc và chạy test nhưng không có quyền sửa, nên nó không thể tự duyệt
    ///    thứ nó đang review. `qa` rơi vào nhánh này vì `validate_rbac` đã chặn
    ///    việc nó giữ `dev-write`.
    /// 3. Còn lại → [`AgentRelation::Manages`].
    ///
    /// [`AgentRelation`]: crate::router::AgentRelation
    #[must_use]
    pub fn relation(&self) -> AgentRelation {
        let tags = self.tag_set();
        if tags.contains(INFRA_SCAN_TAG) {
            return AgentRelation::AlertsDirectly;
        }
        let can_write = tags.contains(DEV_WRITE_TAG);
        let inspects = tags.contains(DEV_READ_TAG) || tags.contains(TEST_RUN_TAG);
        if !can_write && inspects {
            return AgentRelation::Reviews;
        }
        AgentRelation::Manages
    }
}

/// Một project profile trong `[[projects]]` (M21.1).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Tên project (kebab-case), dùng làm khoá chọn project khi mở phiên.
    pub name: String,
    /// Workspace riêng của project; mọi thao tác file của project bị jail trong đây.
    pub workspace: PathBuf,
}

/// Tên project mặc định — ánh xạ tới `agent.workspace` (M21.1).
pub const DEFAULT_PROJECT: &str = "default";

/// `[data]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DataConfig {
    /// Nơi chứa SQLite, audit log, `auth.toml`.
    pub dir: PathBuf,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("~/.BeanAgent"),
        }
    }
}

/// `[llm]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmConfig {
    /// Provider đang dùng.
    pub provider: LlmProviderKind,
    /// Model đang dùng.
    pub model: String,
    /// Danh sách model được phép cho `/model` (mục 10). Rỗng ⇒ chỉ `model`.
    pub allowed_models: Vec<String>,
    /// Tên biến môi trường chứa API key (**không** phải key).
    pub api_key_env: String,
    /// Ghi đè base URL (dùng cho endpoint không phải OpenAI chính thức / Ollama).
    pub base_url: Option<String>,
    /// Trần token cho mỗi lượt gọi.
    pub max_tokens: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProviderKind::Anthropic,
            model: "claude-sonnet-5".to_string(),
            allowed_models: Vec::new(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: None,
            max_tokens: 4096,
        }
    }
}

impl LlmConfig {
    /// Danh sách model mà `/model` được phép chuyển sang.
    #[must_use]
    pub fn effective_allowed_models(&self) -> Vec<String> {
        if self.allowed_models.is_empty() {
            vec![self.model.clone()]
        } else {
            self.allowed_models.clone()
        }
    }
}

/// `[tools.web_search]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebSearchConfig {
    /// Provider tìm kiếm.
    pub provider: WebSearchProvider,
    /// Tên biến môi trường chứa API key (không dùng khi `provider = "searxng"`).
    pub api_key_env: String,
    /// Base URL của SearXNG tự host.
    pub base_url: Option<String>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: WebSearchProvider::Tavily,
            api_key_env: "TAVILY_API_KEY".to_string(),
            base_url: None,
        }
    }
}

/// Cấu hình learning loop sau run thành công (agents.md mục 17, milestone M15).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearningConfig {
    /// Bật reflection và tạo đề xuất skill.
    pub enabled: bool,
    /// Số tool call tối thiểu trong một run trước khi reflection.
    pub min_tool_calls: u32,
    /// Khoảng cách tối thiểu giữa hai đề xuất, tính bằng phút.
    pub proposal_interval_minutes: u64,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_tool_calls: 5,
            proposal_interval_minutes: 60,
        }
    }
}

/// `[tools]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// Nhóm tool bật lên (xem [`KNOWN_TOOL_GROUPS`]).
    pub enabled: Vec<String>,
    /// Cấu hình riêng cho `web_search`.
    pub web_search: WebSearchConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: KNOWN_TOOL_GROUPS.iter().map(|s| (*s).to_string()).collect(),
            web_search: WebSearchConfig::default(),
        }
    }
}

/// `[security.sandbox]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxConfig {
    /// `docker` (mặc định) hoặc `host`.
    pub mode: SandboxMode,
    /// Image sandbox (Debian slim, **không** kèm Node — agents.md mục 15.2).
    pub image: String,
    /// Cho container ra mạng hay không (mặc định: không).
    pub network: bool,
    /// Trần bộ nhớ (`docker --memory`).
    pub memory: String,
    /// Số CPU được dùng.
    pub cpus: f32,
    /// Trần số tiến trình trong container (`docker --pids-limit`, chống fork bomb).
    pub pids_limit: u32,
    /// Thời gian tối đa cho một lệnh shell (giây).
    pub timeout_seconds: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::Docker,
            image: "BeanAgent-sandbox:latest".to_string(),
            network: false,
            memory: "512m".to_string(),
            cpus: 1.0,
            pids_limit: 64,
            timeout_seconds: 60,
        }
    }
}

/// `[security]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    /// Ngân sách token mỗi ngày (mục 15.9).
    pub daily_token_budget: u64,
    /// Trần thời gian cho mỗi tool call (giây, agents.md mục 6).
    pub tool_timeout_seconds: u64,
    /// Cấu hình sandbox cho `run_shell`.
    pub sandbox: SandboxConfig,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            daily_token_budget: 2_000_000,
            tool_timeout_seconds: 60,
            sandbox: SandboxConfig::default(),
        }
    }
}

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
    /// `beanagent-web`, nên mọi phiên web đều là một user duy nhất và RBAC ở tầng
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

/// Trần cho [`McpServerConfig::call_timeout_seconds`] (giây).
///
/// Một giờ là trần có chủ đích: tool chạy lâu thực sự (truy vấn SIEM, xuất báo cáo, quét
/// hạ tầng…) cần hơn 60 giây, nhưng một MCP server treo vô hạn sẽ giữ cả slot của agent
/// loop (`max_steps`) — lỗi thật sự nằm ở chỗ không có trần.
pub const MAX_MCP_CALL_TIMEOUT_SECONDS: u32 = 3600;

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
    /// `None` ⇒ dùng mặc định 60 giây của [`McpTimeouts`](https://docs.rs/beanagent-tools).
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

/// Tiền tố danh tính của client MCP (M25) — **tách biệt hoàn toàn** khỏi `telegram:`/`web:`.
///
/// `Router::authorize` yêu cầu `user_id` bắt đầu bằng `<channel>:`; MCP client đi qua
/// cổng riêng nên identity này được dựng từ `[[mcp_clients]].name`.
pub const MCP_CLIENT_PREFIX: &str = "mcp-client:";

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
    /// SQLite (`data.dir/beanagent.db`) nên `BeanAgent.toml` — thường được commit —
    /// không bao giờ chứa bí mật (xem `Store::create_mcp_client`).
    pub role: String,
}

/// `[mcp_server]` — Bean đóng vai **MCP server** read-only (M25).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerConfigSettings {
    /// Bật chế độ MCP server. Mặc định `false` (bề mặt tấn công mới, phải bật tường minh).
    pub enabled: bool,
    /// Bật transport HTTP (streamable-HTTP/SSE). stdio luôn khả dụng qua `BeanAgent mcp`.
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
    /// loopback (xem `beanagent-browser::guard`).
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

/// Cấu hình gốc của BeanAgent (`BeanAgent.toml`).
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

impl Config {
    /// Đọc `BeanAgent.toml` từ `path`, parse và kiểm tra.
    ///
    /// # Errors
    /// * [`ConfigError::Read`] khi không đọc được file.
    /// * [`ConfigError::Parse`] khi TOML sai cú pháp/sai kiểu/khoá lạ.
    /// * [`ConfigError::Invalid`] khi cấu hình vô nghĩa hoặc không an toàn.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut config: Self = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Nạp cấu hình từ `path` nếu có; nếu không thì dùng giá trị mặc định (có cảnh báo).
    ///
    /// Đây là đường đi của `BeanAgent chat` khi người dùng chưa tạo `BeanAgent.toml`
    /// (`docs/decisions.md` D5.11).
    ///
    /// # Errors
    /// Như [`Config::load`].
    pub fn load_or_default(path: Option<&Path>) -> Result<Self, ConfigError> {
        match path {
            Some(path) => Self::load(path),
            None => {
                tracing::warn!(
                    "chưa có BeanAgent.toml — dùng giá trị mặc định; xem BeanAgent.example.toml để cấu hình đầy đủ"
                );
                let mut config = Self::default();
                config.validate()?;
                Ok(config)
            }
        }
    }

    /// Kiểm tra ngữ nghĩa, chuẩn hoá `~` và `web.public_origin`.
    ///
    /// Những gì **không** kiểm ở đây: sự tồn tại của `data.dir`/`workspace` (tạo ở M3/M5),
    /// tính hợp lệ của múi giờ IANA (cần `chrono-tz`, cài ở M13), và sự tồn tại của API key
    /// (kiểm khi gọi [`Config::resolve_secrets_with`]).
    ///
    /// # Errors
    /// Trả [`ConfigError::Invalid`] khi cấu hình vô nghĩa hoặc không an toàn.
    pub fn validate(&mut self) -> Result<(), ConfigError> {
        self.validate_core()?;
        self.validate_rbac()?;
        self.validate_billing()?;
        self.validate_marketing()?;
        self.validate_infra_scope()?;
        self.validate_projects()?;
        self.validate_web_and_channels()?;
        self.validate_browser()?;
        self.validate_mcp_servers()?;
        self.validate_mcp_servers()?;
        self.validate_mcp_server()?;
        self.validate_paths()
    }
}

impl Config {
    /// RBAC có đang bật không? (M21.3)
    ///
    /// Chỉ bật khi người dùng khai báo ít nhất một `agent.user_roles`. Nếu không, mọi user
    /// trong `allowed_users` giữ hành vi cũ (thấy mọi tool) — nhờ vậy cài đặt một-người-dùng
    /// sẵn có **không** đột nhiên mất tool (D10.3).
    #[must_use]
    pub fn rbac_enabled(&self) -> bool {
        !self.agent.user_roles.is_empty()
    }

    /// **Điểm quyết định RBAC duy nhất** (M21.3): `user_id` → [`RolePermissions`].
    ///
    /// Router gọi hàm này **một lần** mỗi run rồi truyền struct kết quả (tuần tự hoá được)
    /// xuống agent loop; agent loop không tự tra cứu role ⇒ không có logic RBAC rải rác.
    ///
    /// * RBAC tắt ⇒ [`RolePermissions::unrestricted`] (giữ hành vi cũ).
    /// * RBAC bật mà user có trong `user_roles` ⇒ tag của role đó.
    /// * RBAC bật mà user **không** có trong map ⇒ `no-access` (deny-all).
    /// * `user_roles` trỏ tới role không tồn tại ⇒ `no-access`; `validate()` đã chặn từ lúc
    ///   nạp config nên nhánh này chỉ là lưới an toàn.
    #[must_use]
    pub fn permissions_for(&self, user_id: &str) -> RolePermissions {
        if !self.rbac_enabled() {
            return RolePermissions::unrestricted("default");
        }
        let Some(role_name) = self.agent.user_roles.get(user_id) else {
            tracing::warn!(
                user_id,
                "user không có trong agent.user_roles — dùng role no-access (không thấy tool nào)"
            );
            return RolePermissions::deny_all(NO_ACCESS_ROLE);
        };
        match self.role(role_name) {
            Some(role) => {
                RolePermissions::restricted_to(&role.name, role.tag_set(), role.allowed_tag_set())
            }
            None => {
                tracing::error!(user_id, role = %role_name,
                    "user_roles trỏ tới role không tồn tại — dùng no-access");
                RolePermissions::deny_all(NO_ACCESS_ROLE)
            }
        }
    }

    /// Tra role theo tên trong `[[roles]]`.
    #[must_use]
    pub fn role(&self, name: &str) -> Option<&RoleConfig> {
        self.roles.iter().find(|role| role.name == name)
    }

    /// Tra client MCP theo tên trong `[[mcp_clients]]`.
    #[must_use]
    pub fn mcp_client(&self, name: &str) -> Option<&McpClientConfig> {
        self.mcp_clients
            .iter()
            .find(|client| client.name.trim() == name)
    }

    /// Identity đầy đủ của một client MCP (M25): `mcp-client:<name>`.
    #[must_use]
    pub fn mcp_client_identity(name: &str) -> String {
        format!("{MCP_CLIENT_PREFIX}{name}")
    }

    /// Ngân sách context token **riêng cho role** (M21.7).
    #[must_use]
    pub fn context_budget_for(&self, perms: &RolePermissions) -> u32 {
        self.role(&perms.role)
            .and_then(|role| role.context_budget_tokens)
            .unwrap_or(self.agent.context_budget_tokens)
    }

    /// Ngân sách token/ngày **riêng cho role** (M21.7).
    #[must_use]
    pub fn daily_budget_for(&self, perms: &RolePermissions) -> u64 {
        self.role(&perms.role)
            .and_then(|role| role.daily_token_budget)
            .unwrap_or(self.security.daily_token_budget)
    }

    /// Danh sách project đã khai báo; luôn có ít nhất project `default`.
    ///
    /// `default` ánh xạ tới `agent.workspace` (M21.1) — BeanAgent tự phát triển chính nó là một
    /// project profile **không đặc quyền hơn** project nào khác: cùng cơ chế, cùng jail.
    #[must_use]
    pub fn project_profiles(&self) -> Vec<ProjectConfig> {
        let mut profiles = vec![ProjectConfig {
            name: DEFAULT_PROJECT.to_string(),
            workspace: self.agent.workspace.clone(),
        }];
        for project in &self.projects {
            if project.name != DEFAULT_PROJECT {
                profiles.push(project.clone());
            }
        }
        profiles
    }

    /// Workspace của một project; `None` nếu tên không tồn tại.
    #[must_use]
    pub fn project_workspace(&self, name: &str) -> Option<&Path> {
        if name == DEFAULT_PROJECT {
            return Some(&self.agent.workspace);
        }
        self.projects
            .iter()
            .find(|project| project.name == name)
            .map(|project| project.workspace.as_path())
    }

    /// Kiểm tra bảng `[[roles]]` và `agent.user_roles` (M21.2, M21.6).
    fn validate_rbac(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for role in &self.roles {
            let name = role.name.trim();
            if name.is_empty() {
                return Err(invalid("[[roles]] có role thiếu tên"));
            }
            if name == NO_ACCESS_ROLE {
                return Err(invalid(format!(
                    "`{NO_ACCESS_ROLE}` là role dự phòng của hệ thống (deny-all); không được khai báo lại"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!("[[roles]] có role trùng tên `{name}`")));
            }
            // (M21.6) Nguyên tắc four-eyes kiểm ở tầng code: cấp vừa cấm là lỗi cấu hình,
            // không phải việc để model tự phát hiện lúc chạy.
            let granted = role.tag_set();
            for forbidden in &role.forbid_tags {
                let forbidden = forbidden.trim();
                if granted.contains(forbidden) {
                    return Err(invalid(format!(
                        "role `{name}` vừa được cấp tag `{forbidden}` vừa khai báo nó trong forbid_tags — vi phạm nguyên tắc four-eyes"
                    )));
                }
            }
            if role
                .context_budget_tokens
                .is_some_and(|value| value < 1_000)
            {
                return Err(invalid(format!(
                    "role `{name}`.context_budget_tokens tối thiểu 1000"
                )));
            }
            // (M24) Danh sách trắng phải chứa ít nhất một tag mà role thực sự được cấp,
            // nếu không thì role đó bị chặn khỏi MỌI tool (kể cả untagged) mà không ai
            // hỏi — cấu hình im lặng, rất dễ quên.
            let allowed = role.allowed_tag_set();
            if !allowed.is_empty() && !allowed.iter().any(|tag| granted.contains(tag)) {
                return Err(invalid(format!(
                    "role `{name}` khai allowed_tool_tags nhưng tool_tags không chứa tag nào \
                     trong đó — role sẽ không thấy tool nào"
                )));
            }
            // Wildcard `*` trong danh sách trắng là mâu thuẫn: nó đã là "mọi quyền" ở
            // `tool_tags`, thêm vào đây chỉ làm mờ ý nghĩa.
            if allowed.iter().any(|tag| tag == WILDCARD_TAG) {
                return Err(invalid(format!(
                    "role `{name}`: không cần `{WILDCARD_TAG}` trong allowed_tool_tags — \
                     tag đó đã bỏ qua danh sách trắng; bỏ hẳn trường này nếu không giới hạn"
                )));
            }
        }
        for (user, role) in &self.agent.user_roles {
            if self.role(role).is_none() {
                return Err(invalid(format!(
                    "agent.user_roles[`{user}`] trỏ tới role `{role}` không tồn tại trong [[roles]]"
                )));
            }
        }
        Ok(())
    }

    /// Kiểm tra hai trường mới của `[[mcp_servers]]` — phía Bean là MCP **client**
    /// (mục 16): `call_timeout_seconds` và `inherit_env`.
    ///
    /// Không lặp lại kiểm tra `name`/`command` — [`Self::validate_web_and_channels`]
    /// đã làm việc đó (và với thông điệp riêng). Ở đây chỉ những gì **chỉ** trường mới
    /// mới làm hỏng được.
    ///
    /// Chặn ở **tầng config** (nguyên tắc đã dùng cho `forbid_tags` M21.6) vì cả hai
    /// lỗi dưới đây đều im lặng nếu để tới tầng spawn:
    ///
    /// * `call_timeout_seconds = 0` ⇒ `Duration::ZERO` ⇒ **mọi** tool call bị treo tức
    ///   thì và trả về lỗi, model sẽ tưởng server hỏng rồi thử lại vô tận;
    /// * tên biến sai trong `inherit_env` ⇒ `Command::env` **panic** — tệ hơn lỗi
    ///   (`unwrap` trên dữ liệu bên ngoài, mục 22.11).
    fn validate_mcp_servers(&self) -> Result<(), ConfigError> {
        for server in &self.mcp_servers {
            if let Some(seconds) = server.call_timeout_seconds
                && !(1..=MAX_MCP_CALL_TIMEOUT_SECONDS).contains(&seconds)
            {
                return Err(invalid(format!(
                    "[[mcp_servers]].call_timeout_seconds của `{}` = {seconds} nằm ngoài 1..={MAX_MCP_CALL_TIMEOUT_SECONDS}; \
                     0 sẽ khiến mọi tool call bị treo tức thì",
                    server.name
                )));
            }
            for variable in &server.inherit_env {
                validate_inherited_env_name(&server.name, variable)?;
            }
        }
        Ok(())
    }

    /// Kiểm tra `[[mcp_clients]]` + `[mcp_server]` (M25).
    ///
    /// Chặn ở **tầng config** những cấu hình làm MCP server vô nghĩa hoặc không an toàn, để
    /// không phải "phát hiện khi model tự gọi" (nguyên tắc đã dùng cho `forbid_tags` M21.6):
    ///
    /// * client phải có identity `mcp-client:<name>` trong `agent.user_roles` và role đó
    ///   phải tồn tại — nếu không, `permissions_for()` trả `no-access` và client chỉ
    ///   thấy danh sách rỗng mà không ai hiểu vì sao;
    /// * client **không** được dùng role `admin` (tag `*`) — vai trò "mọi quyền" không có
    ///   nghĩa trên bề mặt MCP read-only, và cho phép nó sẽ khiến người đọc cấu hình tưởng
    ///   rằng client có thể ghi/thực thi (M25 phạm vi cứng);
    /// * HTTP bật mà bind ngoài loopback thì phải `allow_remote = true`.
    fn validate_mcp_server(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for client in &self.mcp_clients {
            let name = client.name.trim();
            if name.is_empty() {
                return Err(invalid("[[mcp_clients]] có client thiếu tên"));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!(
                    "[[mcp_clients]] có client trùng tên `{name}`"
                )));
            }
            if name.contains('/') || name.contains('\\') || name.contains(char::is_whitespace) {
                return Err(invalid(format!(
                    "[[mcp_clients]].name = `{name}` chứa ký tự không hợp lệ (kebab-case, không khoảng trắng hay dấu gạch chéo)"
                )));
            }
            if self.role(&client.role).is_none() {
                return Err(invalid(format!(
                    "[[mcp_clients]].role = `{}` của `{name}` không tồn tại trong [[roles]]",
                    client.role
                )));
            }
            if self
                .role(&client.role)
                .is_some_and(|role| role.tag_set().contains(WILDCARD_TAG))
            {
                return Err(invalid(format!(
                    "[[mcp_clients]].role = `{}` của `{name}` giữ tag `*` (admin) — MCP server \
                     M25 chỉ read-only, cấp mọi quyền ở đây chỉ gây hiểu nhầm",
                    client.role
                )));
            }
            let identity = format!("{MCP_CLIENT_PREFIX}{name}");
            match self.agent.user_roles.get(&identity) {
                None => {
                    return Err(invalid(format!(
                        "thiếu `{identity}` trong agent.user_roles — client MCP `{name}` sẽ \
                         resolve thành no-access và không thấy tool nào"
                    )));
                }
                Some(mapped) if mapped != &client.role => {
                    return Err(invalid(format!(
                        "agent.user_roles[`{identity}`] = `{mapped}` khác [[mcp_clients]].role = `{}`",
                        client.role
                    )));
                }
                Some(_) => {}
            }
        }
        if self.mcp_server.http_enabled
            && !self.mcp_server.bind.ip().is_loopback()
            && !self.mcp_server.allow_remote
        {
            return Err(invalid(
                "[mcp_server].bind ngoài loopback yêu cầu allow_remote = true; nếu public, phải đặt sau reverse proxy TLS/Tailscale/VPN",
            ));
        }
        if self.mcp_server.enabled && self.mcp_clients.is_empty() {
            return Err(invalid(
                "[mcp_server].enabled = true nhưng [[mcp_clients]] rỗng — không client nào được xác thực; \
                 chạy `BeanAgent auth mcp-token add <tên>` để sinh token",
            ));
        }
        // K24: bề mặt HTTP có thể công khai nên phải có trần tần suất. Cấu hình sai
        // (bật HTTP mà đặt 0) phải chết lúc nạp chứ không âm thầm mở đường không giới hạn.
        if self.mcp_server.http_enabled && self.mcp_server.rate_limit_per_minute == 0 {
            return Err(invalid(
                "[mcp_server].http_enabled = true mà rate_limit_per_minute = 0 — transport HTTP là \
                 bề mặt có thể công khai, cần trần tần suất để chặn dò token; đặt 0 chỉ hợp lệ \
                 khi bạn tự chịu trách nhiệm (ví dụ chỉ bind loopback)",
            ));
        }
        Ok(())
    }

    /// Kiểm tra `[billing]` (M22a).
    ///
    /// Trọng tâm là **tách credential**: `api_key_env` của billing phải là biến riêng, không
    /// được trùng với biến của LLM / web search / Telegram. Đây là cách hiện thực hoá yêu cầu
    /// "credential đọc billing phải riêng, quyền tối thiểu chỉ đọc billing" ở tầng code thay
    /// vì chỉ dựa vào quy ước viết trong tài liệu (D13.2).
    fn validate_billing(&self) -> Result<(), ConfigError> {
        if !self.billing.enabled {
            return Ok(());
        }
        if self.billing.api_key_env.trim().is_empty() {
            return Err(invalid("[billing].api_key_env rỗng"));
        }
        // Endpoint chỉ lấy từ cấu hình, nhưng vẫn chặn scheme lạ để không biến `base_url`
        // thành đường đọc file cục bộ nếu cấu hình bị sửa nhầm.
        if let Some(base) = &self.billing.base_url {
            validate_http_url("[billing].base_url", base)?;
        }
        if let Some(suffix) = &self.billing.query_suffix
            && (suffix.contains("://") || suffix.contains('@'))
        {
            return Err(invalid(
                "[billing].query_suffix phải chỉ là tham số truy vấn (vd `?period=30d`), không phải URL",
            ));
        }
        let others: [(&str, &str); 3] = [
            ("llm.api_key_env", &self.llm.api_key_env),
            (
                "tools.web_search.api_key_env",
                &self.tools.web_search.api_key_env,
            ),
            ("telegram.token_env", &self.telegram.token_env),
        ];
        for (field, env) in others {
            if env.trim() == self.billing.api_key_env.trim() {
                return Err(invalid(format!(
                    "[billing].api_key_env không được trùng với {field} (`{env}`): credential đọc billing phải RIÊNG, quyền tối thiểu chỉ đọc billing"
                )));
            }
        }
        Ok(())
    }

    /// Kiểm tra `[browser]` (M26).
    ///
    /// Chặn ở **tầng config** (cùng nguyên tắc `forbid_tags` M21.6) vì ba lỗi dưới
    /// đây đều **im lặng** hoặc nguy hiểm nếu để tới tầng chạy:
    ///
    /// * mẫu origin sai cú pháp ⇒ bị bỏ qua lúc parse ⇒ người dùng tưởng đã mở
    ///   whitelist mà thật ra mọi origin vẫn `Dangerous` (lỗi "thắt chặt", ít nguy
    ///   hiểm hơn, nhưng im lặng thì không ai biết phải sửa);
    /// * wildcard ở label không đầu tiên (`https://dev.*.internal`) ⇒ **không** có
    ///   nghĩa an toàn nào, và dễ khiến người dùng tưởng nó hoạt động;
    /// * `max_image_bytes` bằng 0 ⇒ **mọi** lần chụp đều thất bại, model không hiểu
    ///   vì sao.
    fn validate_browser(&self) -> Result<(), ConfigError> {
        let browser = &self.browser;
        if !browser.enabled {
            // Tắt thì không cần validate whitelist: người dùng có thể để sẵn mẫu
            // đang viết rồi bật sau. Việc sửa đường dẫn Chrome cũng vậy.
            return Ok(());
        }
        for pattern in &browser.allowed_origins {
            if let Err(reason) = validate_origin_pattern(pattern) {
                return Err(invalid(format!(
                    "[browser].allowed_origins chứa mẫu không hợp lệ `{pattern}`: {reason}"
                )));
            }
        }
        if browser.executable_path.chars().count() > 4_096 {
            return Err(invalid(
                "[browser].executable_path quá dài; đây là đường dẫn, không phải nội dung",
            ));
        }
        if browser.max_image_bytes == 0 {
            return Err(invalid(
                "[browser].max_image_bytes = 0 khiến mọi lần chụp đều thất bại; \
                 đặt trần thực tế (mặc định 4 MiB)",
            ));
        }
        if browser.launch_timeout_seconds == 0 {
            return Err(invalid(
                "[browser].launch_timeout_seconds = 0 khiến Chrome không có thời gian \
                 khởi động",
            ));
        }
        Ok(())
    }

    /// Kiểm tra `[marketing]` (M24).
    fn validate_marketing(&self) -> Result<(), ConfigError> {
        let env = self.marketing.api_key_env.trim();
        if env.is_empty() {
            return Err(invalid("[marketing].api_key_env không được để trống"));
        }
        // (D15.3) Credential phải RIÊNG: quyền "chỉ post" không được dùng lại key có
        // quyền rộng hơn (LLM, search, Telegram). Cùng mẫu với M22a — kiểm ở tầng load
        // config, không dựa vào kỷ luật vận hành.
        for (other, what) in [
            (self.llm.api_key_env.trim(), "llm.api_key_env"),
            (
                self.tools.web_search.api_key_env.trim(),
                "tools.web_search.api_key_env",
            ),
            (self.telegram.token_env.trim(), "telegram.token_env"),
        ] {
            if !other.is_empty() && other == env {
                return Err(invalid(format!(
                    "[marketing].api_key_env trùng với {what} — M24 yêu cầu credential riêng, \
                     quyền tối thiểu chỉ post"
                )));
            }
        }
        if self.marketing.text_field.trim().is_empty() {
            return Err(invalid("[marketing].text_field không được để trống"));
        }
        Ok(())
    }

    /// Kiểm tra `[[infra_scope]]` + `[security_scan]` (M23).
    ///
    /// `infra_scope` rỗng **hợp lệ** và nghĩa là "từ chối mọi thứ" (fail-closed), nên
    /// `validate()` không yêu cầu phải có scope.
    fn validate_infra_scope(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for entry in &self.infra_scope {
            let value = entry.value.trim();
            if !seen.insert(value.to_string()) {
                return Err(invalid(format!(
                    "[[infra_scope]] có target trùng lặp `{value}`"
                )));
            }
            if let Err(reason) = validate_scope_value(entry.kind, value) {
                return Err(invalid(format!(
                    "[[infra_scope]] target `{value}` không hợp lệ: {reason}. Chỉ chấp nhận IP (vd 203.0.113.7) hoặc CIDR (vd 192.168.10.0/24), KHÔNG nhận hostname"
                )));
            }
        }

        // (D14.3) Scanner bắt buộc cần mạng: cấm tắt để không tạo cấu hình "bật nhưng vô dụng".
        if !self.security_scan.sandbox.network {
            return Err(invalid(
                "[security_scan].sandbox.network phải là true — scanner không có mạng thì không tới được target trong scope",
            ));
        }
        // (D14.4) Chế độ host phải được bật tường minh, tránh vô tình hạ cấp cách ly.
        if self.security_scan.enabled
            && self.security_scan.sandbox.mode == SandboxMode::Host
            && !self.security_scan.sandbox.allow_host
        {
            return Err(invalid(
                "[security_scan].sandbox.mode = \"host\" cần đặt allow_host = true để xác nhận bạn chấp nhận chạy scanner ngoài container",
            ));
        }
        if self.security_scan.sandbox.timeout_seconds == 0 {
            return Err(invalid("[security_scan].sandbox.timeout_seconds phải > 0"));
        }
        // Cảnh báo cần đủ cặp channel + chat_id, nếu không thì cảnh báo im lặng (rất dễ quên).
        if self.security_scan.enabled {
            let has_channel = !self.security_scan.alert_channel.trim().is_empty();
            let has_chat = !self.security_scan.alert_chat_id.trim().is_empty();
            if has_channel != has_chat {
                return Err(invalid(
                    "[security_scan].alert_channel và alert_chat_id phải khai báo cùng nhau (hoặc cả hai để trống = không gửi cảnh báo)",
                ));
            }
        }
        Ok(())
    }

    /// Kiểm tra `[[projects]]` (M21.1).
    fn validate_projects(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for project in &self.projects {
            let name = project.name.trim();
            if name.is_empty() {
                return Err(invalid("[[projects]] có project thiếu tên"));
            }
            if name == DEFAULT_PROJECT {
                return Err(invalid(format!(
                    "`{DEFAULT_PROJECT}` là project dự phòng ánh xạ tới agent.workspace; không khai báo lại"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!(
                    "[[projects]] có project trùng tên `{name}`"
                )));
            }
        }
        Ok(())
    }

    /// Kiểm tra `[agent]`, `[tools]`, `[llm]`, `[security]`.
    fn validate_core(&mut self) -> Result<(), ConfigError> {
        if !(1..=1000).contains(&self.agent.max_steps) {
            return Err(invalid(format!(
                "agent.max_steps phải trong 1..=1000 (đang là {})",
                self.agent.max_steps
            )));
        }
        if self.agent.context_budget_tokens < 1_000 {
            return Err(invalid(format!(
                "agent.context_budget_tokens quá nhỏ ({}); tối thiểu 1000",
                self.agent.context_budget_tokens
            )));
        }
        if self.agent.timezone.trim().is_empty() {
            return Err(invalid(
                "agent.timezone rỗng — cần tên múi giờ IANA, ví dụ `Asia/Ho_Chi_Minh`",
            ));
        }
        if self.agent.allowed_users.is_empty() {
            return Err(invalid(
                "agent.allowed_users rỗng — sẽ không ai dùng được agent",
            ));
        }

        for group in &self.tools.enabled {
            if !KNOWN_TOOL_GROUPS.contains(&group.as_str()) {
                return Err(invalid(format!(
                    "tools.enabled chứa `{group}` không hợp lệ; chỉ nhận: {}",
                    KNOWN_TOOL_GROUPS.join(", ")
                )));
            }
        }

        let web_enabled = self.tools.enabled.iter().any(|group| group == "web");
        let web_search = &self.tools.web_search;
        if web_enabled
            && web_search.provider.requires_api_key()
            && web_search.api_key_env.trim().is_empty()
        {
            return Err(invalid(
                "tools.web_search.api_key_env rỗng — cần TÊN biến môi trường chứa API key",
            ));
        }
        if web_enabled && let Some(base_url) = web_search.base_url.as_deref() {
            validate_http_url("tools.web_search.base_url", base_url)?;
        }
        if web_enabled
            && web_search.provider == WebSearchProvider::Searxng
            && web_search.base_url.is_none()
        {
            return Err(invalid(
                "tools.web_search.provider = searxng cần tools.web_search.base_url",
            ));
        }
        if self.learning.min_tool_calls == 0 {
            return Err(invalid("learning.min_tool_calls phải > 0"));
        }
        if self.learning.proposal_interval_minutes == 0 {
            return Err(invalid("learning.proposal_interval_minutes phải > 0"));
        }

        if self.llm.model.trim().is_empty() {
            return Err(invalid("llm.model rỗng"));
        }
        if !self
            .llm
            .effective_allowed_models()
            .iter()
            .any(|m| m == &self.llm.model)
        {
            return Err(invalid(format!(
                "llm.model `{}` không nằm trong llm.allowed_models",
                self.llm.model
            )));
        }
        if self.llm.api_key_env.trim().is_empty() {
            return Err(invalid(
                "llm.api_key_env rỗng — cần TÊN biến môi trường chứa API key",
            ));
        }
        if !(1..=1_000_000).contains(&self.llm.max_tokens) {
            return Err(invalid(format!(
                "llm.max_tokens phải trong 1..=1000000 (đang là {})",
                self.llm.max_tokens
            )));
        }
        if let Some(base_url) = self.llm.base_url.as_deref() {
            validate_http_url("llm.base_url", base_url)?;
        }

        if self.security.daily_token_budget == 0 {
            return Err(invalid("security.daily_token_budget phải > 0"));
        }
        let sandbox = &self.security.sandbox;
        if sandbox.image.trim().is_empty() {
            return Err(invalid("security.sandbox.image rỗng"));
        }
        if sandbox.memory.trim().is_empty() {
            return Err(invalid("security.sandbox.memory rỗng — ví dụ `512m`"));
        }
        // NaN cũng phải bị từ chối (vì vậy không dùng `!(cpus > 0.0)` — clippy::neg_cmp_op_on_partial_ord).
        if sandbox.cpus <= 0.0 || !sandbox.cpus.is_finite() {
            return Err(invalid(format!(
                "security.sandbox.cpus phải là số hữu hạn > 0 (đang là {})",
                sandbox.cpus
            )));
        }
        if sandbox.timeout_seconds == 0 {
            return Err(invalid("security.sandbox.timeout_seconds phải > 0"));
        }
        if sandbox.mode == SandboxMode::Host {
            tracing::warn!(
                "security.sandbox.mode = \"host\": mọi lệnh shell sẽ bị coi là Dangerous (agents.md mục 15.2)"
            );
        }

        Ok(())
    }
}

impl Config {
    /// Kiểm tra `[web]`, `[telegram]`, `[[mcp_servers]]`.
    fn validate_web_and_channels(&mut self) -> Result<(), ConfigError> {
        let origin = self.web.public_origin.trim_end_matches('/').to_string();
        validate_http_url("web.public_origin", &origin)?;
        self.web.public_origin = origin;

        if !self.web.bind.ip().is_loopback() && !self.web.allow_remote {
            return Err(invalid(format!(
                "web.bind = `{}` không phải loopback nhưng web.allow_remote = false (agents.md mục 15.7)",
                self.web.bind
            )));
        }
        if self.web.bind.ip().is_loopback() && self.web.allow_remote {
            tracing::warn!(
                "web.allow_remote = true nhưng web.bind vẫn là loopback — kiểm tra lại reverse proxy/Tailscale"
            );
        }
        if self.web.session_ttl_hours == 0 {
            return Err(invalid("web.session_ttl_hours phải > 0"));
        }
        if self.web.trust_proxy {
            tracing::warn!(
                "web.trust_proxy = true: chỉ dùng khi BeanAgent nằm sau reverse proxy tin cậy (D4.3)"
            );
        }

        if self.telegram.enabled {
            if self.telegram.allowed_user_ids.is_empty() {
                return Err(invalid(
                    "telegram.enabled = true nhưng telegram.allowed_user_ids rỗng — allowlist là bắt buộc (mục 13)",
                ));
            }
            for user_id in &self.telegram.allowed_user_ids {
                let identity = format!("telegram:{user_id}");
                if !self
                    .agent
                    .allowed_users
                    .iter()
                    .any(|allowed| allowed == &identity)
                {
                    return Err(invalid(format!(
                        "telegram user {user_id} phải có `{identity}` trong agent.allowed_users để Router cho phép"
                    )));
                }
            }
            if self.telegram.token_env.trim().is_empty() {
                return Err(invalid(
                    "telegram.token_env rỗng — cần TÊN biến môi trường chứa bot token",
                ));
            }
            if self.telegram.rate_limit_per_minute == 0 {
                return Err(invalid("telegram.rate_limit_per_minute phải > 0"));
            }
        }

        let mut names = BTreeSet::new();
        for server in &self.mcp_servers {
            if !is_valid_mcp_name(&server.name) {
                return Err(invalid(format!(
                    "mcp_servers[].name `{}` không hợp lệ — chỉ dùng chữ, số, `_`, `-` (vì trở thành tiền tố tên tool)",
                    server.name
                )));
            }
            if !names.insert(server.name.as_str()) {
                return Err(invalid(format!(
                    "mcp_servers có tên trùng: `{}`",
                    server.name
                )));
            }
            if server.command.trim().is_empty() {
                return Err(invalid(format!(
                    "mcp_servers `{}`: command rỗng",
                    server.name
                )));
            }
        }

        Ok(())
    }
}

impl Config {
    /// Chuẩn hoá đường dẫn (`~` → `$HOME`) sau cùng, để các bước kiểm tra phía trên
    /// luôn so sánh trên giá trị người dùng đã viết.
    fn validate_paths(&mut self) -> Result<(), ConfigError> {
        if self.data.dir.as_os_str().is_empty() {
            return Err(invalid("data.dir rỗng"));
        }
        if self.agent.workspace.as_os_str().is_empty() {
            return Err(invalid("agent.workspace rỗng"));
        }
        self.agent.workspace = expand_tilde(&self.agent.workspace)?;
        self.data.dir = expand_tilde(&self.data.dir)?;
        Ok(())
    }

    /// Đọc secret từ biến môi trường của tiến trình hiện tại.
    ///
    /// Chính sách "fail fast có chừng mực" (D6.4):
    /// * `llm.api_key_env` — bắt buộc **chỉ khi** provider cần key (`anthropic` luôn cần;
    ///   `openai_compat` cần trừ khi `base_url` trỏ tới server tự host);
    /// * `tools.web_search.api_key_env` — **không** bắt buộc (chỉ cảnh báo khi nhóm `web`
    ///   bật mà thiếu; lỗi rõ ràng sẽ nổi tại lúc tool được gọi — M7);
    /// * `telegram.token_env` — bắt buộc khi `telegram.enabled`.
    ///
    /// # Errors
    /// Trả [`ConfigError::MissingEnv`] khi secret bắt buộc chưa được đặt (nêu rõ **tên biến**
    /// và **trường cấu hình** khai báo nó, không bao giờ nêu giá trị).
    pub fn resolve_secrets(&self) -> Result<ResolvedSecrets, ConfigError> {
        self.resolve_secrets_with(|name| std::env::var(name).ok())
    }

    /// Read only the Telegram bot token from its configured environment variable.
    ///
    /// This is separate from [`Config::resolve_secrets`] so `serve --fake-llm`
    /// can start Telegram without requiring an unused LLM API key.
    ///
    /// # Errors
    /// Returns [`ConfigError::MissingEnv`] when Telegram is enabled and its
    /// configured environment variable is absent or blank.
    pub fn resolve_telegram_token(&self) -> Result<Option<SecretString>, ConfigError> {
        if !self.telegram.enabled {
            return Ok(None);
        }
        let token = read_required(
            &|name: &str| std::env::var(name).ok(),
            "telegram.token_env",
            &self.telegram.token_env,
        )?;
        Ok(Some(token))
    }

    /// Đọc credential billing (M22a).
    ///
    /// Trả `None` khi billing tắt. Khi billing **bật** mà biến chưa được đặt thì trả lỗi
    /// rõ ràng — không âm thầm chạy stub, vì người vận hành cần biết ngay là chưa có key
    /// thay vì tưởng đã đọc được chi phí.
    ///
    /// # Errors
    /// [`ConfigError::MissingEnv`] kể billing bật mà biến `api_key_env` thiếu/rỗng.
    pub fn resolve_billing_key(&self) -> Result<Option<SecretString>, ConfigError> {
        if !self.billing.enabled {
            return Ok(None);
        }
        let key = read_required(
            &|name: &str| std::env::var(name).ok(),
            "billing.api_key_env",
            &self.billing.api_key_env,
        )?;
        Ok(Some(key))
    }

    /// Đọc credential publish của marketing (M24).
    ///
    /// Giống [`Self::resolve_billing_key`]: marketing bật mà thiếu biến thì **báo lỗi lúc
    /// khởi động**, không âm thầm chạy stub — vì `marketing_publish` là hành động không hoàn
    /// tác, người vận hành phải biết chắc là mình đã cấu hình.
    ///
    /// # Errors
    /// [`ConfigError::MissingEnv`] kể marketing bật mà biến `api_key_env` thiếu/rỗng.
    pub fn resolve_marketing_key(&self) -> Result<Option<SecretString>, ConfigError> {
        if !self.marketing.enabled {
            return Ok(None);
        }
        let key = read_required(
            &|name: &str| std::env::var(name).ok(),
            "marketing.api_key_env",
            &self.marketing.api_key_env,
        )?;
        Ok(Some(key))
    }

    /// Read optional API key for `web_search` from biến môi trường được cấu hình.
    ///
    /// Trả `None` khi nhóm `web` tắt, provider không cần key (SearXNG), hoặc biến chưa
    /// được đặt. Việc thiếu key không chặn khởi động; `web_search` sẽ báo rõ khi được gọi.
    #[must_use]
    pub fn resolve_web_search_api_key(&self) -> Option<SecretString> {
        self.resolve_web_search_api_key_with(&|name| std::env::var(name).ok())
    }

    fn resolve_web_search_api_key_with<P>(&self, get_env: &P) -> Option<SecretString>
    where
        P: Fn(&str) -> Option<String>,
    {
        let enabled = self.tools.enabled.iter().any(|group| group == "web");
        if enabled && self.tools.web_search.provider.requires_api_key() {
            read_optional(get_env, &self.tools.web_search.api_key_env)
        } else {
            None
        }
    }

    /// Như [`Config::resolve_secrets`] nhưng nhận hàm đọc biến môi trường tuỳ ý.
    ///
    /// Dùng cho test để không phải `set_var` (trong Rust 2024 `set_var` là `unsafe` và sẽ
    /// phá vỡ `#![forbid(unsafe_code)]` của workspace).
    ///
    /// # Errors
    /// Như [`Config::resolve_secrets`].
    pub fn resolve_secrets_with<P>(&self, get_env: P) -> Result<ResolvedSecrets, ConfigError>
    where
        P: Fn(&str) -> Option<String>,
    {
        let llm_key = read_optional(&get_env, &self.llm.api_key_env);
        // Fail-fast chỉ khi provider thực sự cần key:
        // * `anthropic` — luôn cần;
        // * `openai_compat` — cần trừ khi `base_url` trỏ tới server tự host (Ollama/vLLM).
        // Lỗi khác "thiếu key" (ví dụ endpoint từ chối 401) sẽ nổi lên khi gọi API.
        let llm_key_needed = match self.llm.provider {
            LlmProviderKind::Anthropic => true,
            LlmProviderKind::OpenAiCompat => self.llm.base_url.is_none(),
        };
        if llm_key_needed && llm_key.is_none() {
            return Err(ConfigError::MissingEnv {
                field: "llm.api_key_env",
                env: self.llm.api_key_env.clone(),
            });
        }

        // Key web_search **không** bắt buộc lúc khởi động: `chat` phải dùng được mà không
        // cần Tavily (D6.4). Khi nhóm `web` bật mà thiếu key thì chỉ cảnh báo; M7 sẽ trả
        // lỗi rõ ràng ngay tại lúc tool `web_search` được gọi mà không có key.
        let web_tools_enabled = self.tools.enabled.iter().any(|group| group == "web");
        let web_search_api_key = self.resolve_web_search_api_key_with(&get_env);
        if web_tools_enabled
            && self.tools.web_search.provider.requires_api_key()
            && web_search_api_key.is_none()
        {
            tracing::warn!(
                env = %self.tools.web_search.api_key_env,
                "nhóm tool `web` đang bật nhưng chưa đặt biến môi trường cho key tìm kiếm — tool `web_search` sẽ báo lỗi khi được gọi"
            );
        }

        let telegram_token = if self.telegram.enabled {
            Some(read_required(
                &get_env,
                "telegram.token_env",
                &self.telegram.token_env,
            )?)
        } else {
            None
        };

        Ok(ResolvedSecrets {
            llm_api_key: llm_key,
            web_search_api_key,
            telegram_token,
        })
    }
}

/// Đọc env thành secret; trả `None` khi biến chưa đặt hoặc giá trị chỉ toàn khoảng trắng.
fn read_optional<P>(get_env: &P, env: &str) -> Option<SecretString>
where
    P: Fn(&str) -> Option<String>,
{
    get_env(env)
        .filter(|value| !value.trim().is_empty())
        .map(SecretString::from)
}

/// Đọc env thành secret; lỗi [`ConfigError::MissingEnv`] khi thiếu (nêu tên biến + trường).
fn read_required<P>(
    get_env: &P,
    field: &'static str,
    env: &str,
) -> Result<SecretString, ConfigError>
where
    P: Fn(&str) -> Option<String>,
{
    read_optional(get_env, env).ok_or_else(|| ConfigError::MissingEnv {
        field,
        env: env.to_string(),
    })
}

/// Kiểm tra cú pháp một giá trị trong `[[infra_scope]]`.
///
/// Trả `Err(mô tả lỗi)` nếu sai. Cố ý **không** chấp nhận hostname: xem
/// [`ScanTargetKind`].
pub fn validate_scope_value(kind: ScanTargetKind, value: &str) -> Result<(), &'static str> {
    use std::net::IpAddr;
    match kind {
        ScanTargetKind::Ip => match value.parse::<IpAddr>() {
            Ok(_) => Ok(()),
            Err(_) => Err("không phải địa chỉ IP hợp lệ"),
        },
        ScanTargetKind::Cidr => {
            let Some((addr, len)) = value.split_once('/') else {
                return Err("thiếu hậu tố /len");
            };
            let Ok(ip) = addr.parse::<IpAddr>() else {
                return Err("phần địa chỉ không hợp lệ");
            };
            let Ok(len) = len.parse::<u8>() else {
                return Err("độ dài tiền tố /len không phải số");
            };
            let max = if ip.is_ipv4() { 32 } else { 128 };
            if len > max {
                return Err("độ dài tiền tố vượt giới hạn của họ địa chỉ");
            }
            Ok(())
        }
    }
}

/// Tạo lỗi cấu hình sai.
fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}

/// Kiểm tra một mẫu trong `[browser].allowed_origins` (M26).
///
/// Cùng luật với [`beanagent-browser`](crate::config) nhưng **viết lại ở đây**,
/// không import từ crate browser: chiều phụ thuộc phải là `types` → `browser`, không
/// bao giờ ngược lại (nếu không, mọi thứ dùng `Config` cũng phải kéo `chromiumoxide`
/// vào). Hai bản phải giữ cùng quy tắc — test ở `beanagent-browser` chốt hành vi thật.
fn validate_origin_pattern(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("mẫu rỗng".into());
    }
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err("thiếu scheme, phải có dạng `scheme://host[:port]`".into());
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
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
            if parsed == 0 {
                return Err("port 0 không hợp lệ".into());
            }
            (host, parsed)
        }
        None => (rest, 0),
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
    if host.trim_start_matches("*.").is_empty() {
        return Err("mẫu wildcard `*` trần quá rộng; hãy ghi domain cụ thể".into());
    }
    let _ = port;
    if !host.starts_with('*') {
        let bare = host.trim_matches(|c| c == '[' || c == ']');
        let looks_like_ip = bare.parse::<std::net::IpAddr>().is_ok();
        if !looks_like_ip
            && !bare.split('.').all(|label| {
                !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        {
            return Err(format!("host `{host}` không phải tên miền hợp lệ"));
        }
    }
    Ok(())
}

/// Kiểm tra một tên biến trong `[[mcp_servers]].inherit_env`.
///
/// `Command::env` **panic** khi tên rỗng hoặc chứa `=`/NUL, và im lặng bỏ qua tên có
/// ký tự lạ trên một số nền tảng. Vì đây là dữ liệu từ file cấu hình của người dùng,
/// phải chết sớm ở tầng config với thông điệp chỉ đường thay vì sập tiến trình.
///
/// Chỉ chấp nhận `[A-Za-z_][A-Za-z0-9_]*` — đúng POSIX, và là tập con của những gì
/// mọi hệ điều hành Bean chạy trên chấp nhận cho tên biến.
fn validate_inherited_env_name(server: &str, variable: &str) -> Result<(), ConfigError> {
    let name = variable.trim();
    if name.is_empty() {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên biến rỗng"
        )));
    }
    if name.contains('=') || name.contains('\0') {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên `{name}` chứa '=' hoặc NUL — không phải tên biến hợp lệ"
        )));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên `{name}` chứa ký tự ngoài [A-Za-z0-9_]"
        )));
    }
    if name.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên `{name}` bắt đầu bằng chữ số"
        )));
    }
    Ok(())
}

/// Kiểm tra URL `http(s)` ở mức tối thiểu (M1 chưa cần crate `url`).
fn validate_http_url(field: &str, value: &str) -> Result<(), ConfigError> {
    let lowered = value.to_ascii_lowercase();
    let rest = lowered
        .strip_prefix("https://")
        .or_else(|| lowered.strip_prefix("http://"))
        .ok_or_else(|| {
            invalid(format!(
                "{field} phải bắt đầu bằng `http://` hoặc `https://` (đang là `{value}`)"
            ))
        })?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(invalid(format!("{field} thiếu host (đang là `{value}`)")));
    }
    Ok(())
}

/// Tên MCP server: chỉ chữ/số/`_`/`-` vì được ghép vào tên tool `mcp__<server>__<tool>`.
fn is_valid_mcp_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Mở rộng `~`/`~/...` thành `$HOME` (agents.md mục 18: `data.dir = "~/.BeanAgent"`).
fn expand_tilde(path: &Path) -> Result<PathBuf, ConfigError> {
    let raw = path.to_string_lossy();
    if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| invalid("không xác định được $HOME để mở rộng `~` trong đường dẫn"))?;
        let rest = raw.strip_prefix('~').unwrap_or_default();
        return Ok(PathBuf::from(home).join(rest.trim_start_matches('/')));
    }
    Ok(path.to_path_buf())
}
