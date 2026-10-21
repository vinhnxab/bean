//! Hợp đồng công khai của Router: [`Channel`], [`Incoming`], [`RouterError`],
//! tùy chọn runtime, dependency và các snapshot trả về adapter.

use super::*;

/// Adapter nhận input và gửi output; không chứa logic agent.
#[async_trait::async_trait]
pub trait Channel: Send + Sync {
    /// Tên đăng ký trong Router.
    fn name(&self) -> &'static str;

    /// Chạy input loop cho tới khi `shutdown` được huỷ.
    async fn run(&self, router: Arc<Router>, shutdown: CancellationToken) -> AnyResult<()>;

    /// Gửi một tin chủ động đã lưu trong DB.
    async fn send(&self, chat_id: &str, out: Outbound) -> AnyResult<()>;
}

/// Tin từ channel chuyển vào Router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    /// Channel nhận tin.
    pub channel: String,
    /// Chat ID định danh hội thoại.
    pub chat_id: String,
    /// User ID đầy đủ, ví dụ `cli:local`.
    pub user_id: String,
    /// Nội dung hoặc slash command.
    pub text: String,
    /// Session cụ thể; `None` resolve theo `(channel, chat_id, user_id)`.
    pub session_id: Option<SessionId>,
    /// Project profile muốn dùng (M21.1); `None` ⇒ project mặc định.
    pub project: Option<String>,
    /// Lượt này bắt nguồn từ giọng nói push-to-talk (M27).
    ///
    /// `true` ⇒ Router giới hạn lượt chỉ chạy tool `Risk::Safe` (xem
    /// [`bean_types::RolePermissions::restrict_to_safe_only`]) và chèn hướng dẫn trả lời
    /// ngắn vào system prompt. Adapter web đặt cờ này khi tin nhắn đến từ nút mic.
    pub voice: bool,
}

impl Incoming {
    /// Tạo tin không chỉ định session.
    #[must_use]
    pub fn new(
        channel: impl Into<String>,
        chat_id: impl Into<String>,
        user_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            chat_id: chat_id.into(),
            user_id: user_id.into(),
            text: text.into(),
            session_id: None,
            project: None,
            voice: false,
        }
    }

    /// Gắn session cụ thể (adapter web dùng để chống IDOR).
    #[must_use]
    pub fn with_session(mut self, session: SessionId) -> Self {
        self.session_id = Some(session);
        self
    }

    /// Gắn project profile (M21.1).
    #[must_use]
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Đánh dấu lượt là giọng nói push-to-talk (M27).
    #[must_use]
    pub fn with_voice(mut self, voice: bool) -> Self {
        self.voice = voice;
        self
    }
}

/// Lỗi quyết định của Router.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    /// User không nằm trong `agent.allowed_users`.
    #[error("người dùng không được phép: {0}")]
    Forbidden(String),
    /// User ID không mang prefix của channel.
    #[error("user_id không thuộc channel {channel}")]
    InvalidIdentity {
        /// Channel bị từ chối.
        channel: String,
    },
    /// Project profile không tồn tại trong cấu hình (M21.1).
    #[error("project `{0}` không tồn tại trong cấu hình")]
    InvalidProject(String),
    /// Lỗi store.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Lỗi skill/draft.
    #[error(transparent)]
    Skill(#[from] SkillError),
    /// Lỗi nội bộ không chứa secret.
    #[error("lỗi nội bộ: {0}")]
    Internal(String),
    /// Session không tồn tại hoặc không thuộc user/channel/chat.
    #[error("session không hợp lệ cho user/channel/chat")]
    InvalidSession,
    /// `/new` khi session còn run hoặc queue.
    #[error("session còn run đang hoạt động; hãy dùng /stop trước /new")]
    SessionBusy,
    /// Slash command chưa có trong M8.
    #[error("slash command chưa hỗ trợ: {0}")]
    UnknownCommand(String),
    /// Model không được phép chuyển runtime.
    #[error("model không được phép: {0}")]
    ModelNotAllowed(String),
    /// Confirm đã resolve hoặc không tồn tại.
    #[error("confirm không tồn tại hoặc đã được phân giải")]
    ConfirmNotFound,
    /// Confirm thuộc user khác.
    #[error("actor không được phân giải confirm của user khác")]
    ConfirmForbidden,
    /// Channel đã đăng ký.
    #[error("channel đã đăng ký: {0}")]
    DuplicateChannel(String),
    /// Không có Tokio runtime khi start worker.
    #[error("Router cần Tokio runtime để start worker outbox")]
    NoRuntime,
    /// State nội bộ bị poison.
    #[error("state nội bộ Router không khả dụng")]
    StatePoisoned,
    /// OS không cấp được entropy.
    #[error("không sinh được ID ngẫu nhiên: {0}")]
    Random(String),
    /// (M25) Client MCP gọi thẳng tool ngoài danh sách được expose.
    ///
    /// Tách khỏi [`Self::Forbidden`] vì đây không phải "sai vai trò" mà là **gọi thẳng
    /// tên tool không thuộc bề mặt MCP** — `Plan.md` M25 yêu cầu phải *từ chối*, chứ không
    /// được chỉ ẩn khỏi `tools/list`.
    #[error("tool `{0}` không được expose qua MCP server")]
    ToolNotExposed(String),
    /// Profile provider không tồn tại trong cấu hình (hot-swap provider).
    #[error("profile provider không tồn tại: {0}")]
    ProfileNotFound(String),
    /// Dựng provider LLM mới thất bại (thiếu key, sai base_url…). Thông điệp lấy từ
    /// [`bean_llm::LlmError`] — chỉ nêu tên biến môi trường, không bao giờ chứa giá trị key.
    #[error("không dựng được provider LLM: {0}")]
    LlmProvider(String),
    /// Không ghi được file trạng thái LLM — switch bị từ chối nguyên vẹn (config không đổi).
    #[error("không ghi được trạng thái LLM: {0}")]
    Persist(String),
    /// Profile mới/sửa không đạt validate (tên, model, api_key_env…).
    #[error("profile không hợp lệ: {0}")]
    ProfileInvalid(String),
    /// Tên profile đã tồn tại trong `[[llm.providers]]`.
    #[error("tên profile đã tồn tại: {0}")]
    ProfileExists(String),
    /// Profile đang active — phải switch sang profile khác trước khi xoá.
    #[error("profile đang hoạt động, không thể xoá: {0}")]
    ProfileActive(String),
    /// Profile `"default"` là cấu hình `[llm]` ngầm định, không sửa/xoá được qua API.
    #[error("profile `default` là cấu hình [llm] ngầm định, không sửa/xoá được")]
    DefaultProfileReadonly,
    /// Không ghi được `bean.toml` — CRUD profile bị từ chối nguyên vẹn.
    #[error("không ghi được file cấu hình: {0}")]
    ConfigPersist(String),
}

impl RouterError {
    /// Mã lỗi ổn định cho adapter, không chứa nội dung bí mật.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Forbidden(_) | Self::InvalidIdentity { .. } => "forbidden",
            Self::InvalidProject(_) => "invalid_project",
            Self::Store(_) => "store",
            Self::Skill(_) => "skill_draft",
            Self::Internal(_) => "internal",
            Self::InvalidSession => "invalid_session",
            Self::SessionBusy => "session_busy",
            Self::UnknownCommand(_) => "unknown_command",
            Self::ModelNotAllowed(_) => "model_not_allowed",
            Self::ConfirmNotFound => "confirm_not_found",
            Self::ConfirmForbidden => "confirm_forbidden",
            Self::DuplicateChannel(_) => "duplicate_channel",
            Self::NoRuntime => "no_runtime",
            Self::StatePoisoned => "router_state",
            Self::Random(_) => "random_unavailable",
            Self::ToolNotExposed(_) => "tool_not_exposed",
            Self::ProfileNotFound(_) => "profile_not_found",
            Self::LlmProvider(_) => "provider_build_failed",
            Self::Persist(_) => "llm_state_unwritable",
            Self::ProfileInvalid(_) => "profile_invalid",
            Self::ProfileExists(_) => "profile_exists",
            Self::ProfileActive(_) => "profile_active",
            Self::DefaultProfileReadonly => "default_readonly",
            Self::ConfigPersist(_) => "config_unwritable",
        }
    }
}

/// Tùy chọn runtime, test có thể rút ngắn timeout/backoff.
#[derive(Debug, Clone)]
pub struct RouterOptions {
    /// Sức chứa broadcast.
    pub event_capacity: usize,
    /// Timeout confirm (mặc định 300 giây).
    pub confirm_timeout: Duration,
    /// Chu kỳ quét outbox.
    pub outbox_poll_interval: Duration,
    /// Backoff lần đầu.
    pub outbox_base_delay: Duration,
    /// Trần backoff.
    pub outbox_max_delay: Duration,
}

impl Default for RouterOptions {
    fn default() -> Self {
        Self {
            event_capacity: DEFAULT_EVENT_CAPACITY,
            confirm_timeout: Duration::from_secs(300),
            outbox_poll_interval: Duration::from_secs(1),
            outbox_base_delay: Duration::from_secs(1),
            outbox_max_delay: Duration::from_secs(300),
        }
    }
}

/// Dependency cần thiết để Router chạy agent.
pub struct RouterDeps {
    /// Cấu hình agent.
    pub config: Config,
    /// Store lịch sử/outbox.
    pub store: Arc<dyn Store>,
    /// Registry tool dùng chung.
    pub registry: Arc<ToolRegistry>,
    /// Provider LLM.
    pub llm: Arc<dyn LlmProvider>,
    /// Audit log tùy chọn.
    pub audit: Option<Arc<AuditLog>>,
    /// Index skill `name: description` cho `/skills` và system prompt.
    pub skills_index: String,
    /// Catalog dùng cho learning, duyệt draft và reload sau khi kích hoạt.
    pub skills: Option<SkillCatalog>,
    /// Đường dẫn file persist lựa chọn provider (`data.dir/llm-state.json`).
    ///
    /// `None` = không persist (test); mọi switch vẫn đổi runtime nhưng sẽ mất khi restart.
    pub llm_state_path: Option<std::path::PathBuf>,
    /// Đường dẫn `bean.toml` để CRUD profile provider từ UI ghi lại.
    ///
    /// `None` (test, `--fake-llm`, `chat`) = profile chỉ tồn tại trong RAM; adapter
    /// web hỏi [`Router::llm_profiles_writable`] để trả 503 thay vì giả vờ lưu.
    pub config_path: Option<std::path::PathBuf>,
    /// Snapshot `[llm]` **gốc** từ `bean.toml`, làm profile `"default"`.
    ///
    /// Phải được chụp TRƯỚC khi caller áp trạng thái persist vào `config` — nếu lấy
    /// từ `config` sau đó thì `"default"` sẽ là lựa chọn đã lưu, và switch về
    /// `"default"` thành no-op. Test truyền `config.llm.as_profile()`.
    pub default_profile: bean_types::config::LlmProfile,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunningInfo {
    /// ID run.
    pub run_id: RunId,
    /// Session của run.
    pub session_id: SessionId,
    /// Role sở hữu run; `None` khi RBAC tắt.
    pub role: Option<String>,
}

/// Snapshot phần `[llm]` của cấu hình runtime, đọc qua cùng `RwLock` mà `/model`
/// dùng — adapter web đọc từ đây thay vì giữ bản config tĩnh lúc khởi động.
/// Không chứa giá trị secret: `key_configured` chỉ là cờ "biến môi trường có giá trị".
#[derive(Debug, Clone, PartialEq)]
pub struct LlmRuntimeConfig {
    /// Provider đang dùng.
    pub provider: LlmProviderKind,
    /// Model đang dùng.
    pub model: String,
    /// Danh sách model được phép chuyển runtime.
    pub allowed_models: Vec<String>,
    /// Tên biến môi trường chứa API key (không phải giá trị).
    pub api_key_env: String,
    /// Base URL ghi đè (openai_compat / Ollama).
    pub base_url: Option<String>,
    /// Trần token output của một lượt gọi.
    pub max_tokens: u32,
    /// `true` khi biến môi trường `api_key_env` tồn tại và khác rỗng trong process.
    pub key_configured: bool,
}

/// Một profile provider trong danh sách switch (hot-swap), đã khử secret.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmProfileInfo {
    /// Tên profile (`"default"` là profile ngầm định từ `[llm]` lúc khởi động).
    pub name: String,
    /// Provider LLM.
    pub provider: LlmProviderKind,
    /// Model mặc định của profile.
    pub model: String,
    /// Model được phép chọn trong profile.
    pub allowed_models: Vec<String>,
    /// Base URL ghi đè.
    pub base_url: Option<String>,
    /// Trần token cho mỗi lượt gọi của profile.
    pub max_tokens: u32,
    /// Tên biến môi trường chứa API key (không phải giá trị); rỗng = không cần key.
    pub api_key_env: String,
    /// `true` khi biến môi trường (nếu cần) có giá trị trong process.
    pub key_configured: bool,
    /// 4 ký tự cuối của key đã lưu trực tiếp (vd `"ed6"` bọc thành `"••••ed6"`);
    /// `None` = profile không có key trong `bean.toml` (resolve từ env).
    pub api_key_hint: Option<String>,
    /// `true` khi profile đang có hiệu lực.
    pub active: bool,
}

/// Ý định thay đổi API key của profile khi sửa từ UI — tách bạch ba semantics
/// "giữ nguyên / xoá / thay mới" để form để trống không vô tình xoá key đã lưu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyUpdate {
    /// Body không gửi trường `api_key` ⇒ giữ key hiện tại của profile.
    Keep,
    /// Body gửi chuỗi rỗng ⇒ xoá key đã lưu (quay lại resolve từ env).
    Clear,
    /// Body gửi key mới ⇒ ghi đè key trong `bean.toml`.
    Set(String),
}

/// Confirm đang chờ, dùng để đồng bộ sau reconnect.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingConfirmInfo {
    /// ID confirm.
    pub confirm_id: ConfirmId,
    /// Session của run.
    pub session_id: SessionId,
    /// ID run.
    pub run_id: RunId,
    /// Nội dung hành động.
    pub prompt: String,
    /// Mức rủi ro.
    pub risk: Risk,
    /// Có cho phép allow-in-session hay không.
    pub allow_session_option: bool,
    /// Timeout còn lại/được yêu cầu, tính bằng giây.
    pub timeout_seconds: u32,
    /// Role của agent đang chờ duyệt; `None` khi RBAC tắt.
    pub role: Option<String>,
}

/// Snapshot atomically đọc từ Router state.
#[derive(Debug, Clone, PartialEq)]
pub struct RouterSnapshot {
    /// Các run đang active.
    pub running: Vec<RunningInfo>,
    /// Các confirm đang chờ.
    pub pending_confirms: Vec<PendingConfirmInfo>,
}
