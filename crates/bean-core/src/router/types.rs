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
