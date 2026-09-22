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

/// Tên file cấu hình mặc định ở gốc workspace.
pub const DEFAULT_CONFIG_FILE: &str = "BeanAgent.toml";

/// Nhóm tool hợp lệ cho `[tools] enabled` (agents.md mục 7.3).
pub const KNOWN_TOOL_GROUPS: &[&str] = &["files", "shell", "web", "memory", "skills", "schedule"];

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
        }
    }
}

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
    /// Tin `X-Forwarded-For`/`X-Real-IP` (chỉ khi chạy sau reverse proxy tin cậy — D4.3).
    pub trust_proxy: bool,
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
    /// `[security]`.
    pub security: SecurityConfig,
    /// `[web]`.
    pub web: WebConfig,
    /// `[telegram]`.
    pub telegram: TelegramConfig,
    /// `[[mcp_servers]]`.
    pub mcp_servers: Vec<McpServerConfig>,
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
        self.validate_web_and_channels()?;
        self.validate_paths()
    }
}

impl Config {
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
        let web_search_api_key = read_optional(&get_env, &self.tools.web_search.api_key_env);
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

/// Tạo lỗi cấu hình sai.
fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
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
