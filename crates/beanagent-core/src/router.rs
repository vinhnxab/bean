//! Router, Channel và slash command của lõi (agents.md mục 10).
//!
//! Run thuộc Router: adapter chỉ `submit` và nhận event. Vòng đời run không phụ
//! thuộc subscriber, nên WebSocket rớt không làm cancel tool hoặc confirm.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};
use std::time::Duration;

use anyhow::Result as AnyResult;
use beanagent_llm::LlmProvider;
use beanagent_security::{AuditLog, SessionPolicy};
use beanagent_skills::{SkillCatalog, SkillDraftDecision, SkillError};
use beanagent_tools::{AlertSink, ToolRegistry, truncate_chars};
use beanagent_types::{
    Alert, Config, ConfirmId, ConfirmOutcome, Outbound, OutboundKind, Risk, RolePermissions,
    RunEvent, RunId, SessionId,
};
use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use crate::agent::{AgentError, RunOutcome, RunTurnArgs, run_turn_outcome};
use crate::learning::{ReflectionArgs, reflect};
use crate::run_io::{Decision, RunIo};
use crate::store::{Store, StoreError};

const DEFAULT_EVENT_CAPACITY: usize = 256;
const OUTBOX_BATCH_SIZE: usize = 50;
const PREVIEW_CHARS: usize = 2_000;

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
}

/// Snapshot atomically đọc từ Router state.
#[derive(Debug, Clone, PartialEq)]
pub struct RouterSnapshot {
    /// Các run đang active.
    pub running: Vec<RunningInfo>,
    /// Các confirm đang chờ.
    pub pending_confirms: Vec<PendingConfirmInfo>,
}

#[derive(Debug)]
struct SessionQueue {
    active: Option<RunId>,
    pending: VecDeque<QueuedRun>,
}

#[derive(Debug, Clone)]
struct QueuedRun {
    run_id: RunId,
    session_id: SessionId,
    incoming: Incoming,
    cancel: CancellationToken,
    /// `Some` khi run do scheduler gọi; interactive run là `None`.
    background_allowed_tools: Option<Arc<HashSet<String>>>,
}

#[derive(Debug, Clone)]
struct ActiveRun {
    channel: String,
    chat_id: String,
    cancel: CancellationToken,
}

#[derive(Debug)]
struct PendingConfirm {
    run_id: RunId,
    session_id: SessionId,
    actor: String,
    prompt: String,
    risk: Risk,
    allow_session_option: bool,
    timeout_seconds: u32,
    sender: oneshot::Sender<Confirmation>,
}

#[derive(Debug, Clone)]
struct Confirmation {
    decision: Decision,
    actor: String,
}

#[derive(Debug, Default)]
struct RouterState {
    queues: HashMap<SessionId, SessionQueue>,
    active: HashMap<RunId, ActiveRun>,
    confirms: HashMap<ConfirmId, PendingConfirm>,
    policies: HashMap<SessionId, Arc<SessionPolicy>>,
}

struct RouterInner {
    config: RwLock<Config>,
    store: Arc<dyn Store>,
    registry: Arc<ToolRegistry>,
    llm: Arc<dyn LlmProvider>,
    audit: Option<Arc<AuditLog>>,
    skills: Option<SkillCatalog>,
    skills_index: RwLock<String>,
    learning_gate: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    options: RouterOptions,
    events: broadcast::Sender<RunEvent>,
    channels: RwLock<HashMap<String, Arc<dyn Channel>>>,
    state: Mutex<RouterState>,
    submission: AsyncMutex<()>,
    outbox_started: AtomicBool,
    shutdown: CancellationToken,
}

/// Router sở hữu queue, run, confirm, channel và outbox.
#[derive(Clone)]
pub struct Router {
    inner: Arc<RouterInner>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("event_capacity", &self.inner.options.event_capacity)
            .finish_non_exhaustive()
    }
}

impl Router {
    /// Tạo Router với timeout chuẩn 300 giây.
    #[must_use]
    pub fn new(deps: RouterDeps) -> Self {
        Self::with_options(deps, RouterOptions::default())
    }

    /// Tạo Router với options tường minh (test có thể rút ngắn thời gian).
    #[must_use]
    pub fn with_options(deps: RouterDeps, options: RouterOptions) -> Self {
        let RouterDeps {
            config,
            store,
            registry,
            llm,
            audit,
            skills_index,
            skills,
        } = deps;
        let (events, _) = broadcast::channel(options.event_capacity.max(1));
        Self {
            inner: Arc::new(RouterInner {
                config: RwLock::new(config),
                store,
                registry,
                llm,
                audit,
                skills,
                skills_index: RwLock::new(skills_index),
                learning_gate: Mutex::new(None),
                options,
                events,
                channels: RwLock::new(HashMap::new()),
                state: Mutex::new(RouterState::default()),
                submission: AsyncMutex::new(()),
                outbox_started: AtomicBool::new(false),
                shutdown: CancellationToken::new(),
            }),
        }
    }

    /// Đăng ký adapter gửi outbound.
    pub fn register_channel(&self, channel: Arc<dyn Channel>) -> Result<(), RouterError> {
        let name = channel.name().to_string();
        let mut channels = write_lock(&self.inner.channels)?;
        if channels.contains_key(&name) {
            return Err(RouterError::DuplicateChannel(name));
        }
        channels.insert(name, channel);
        Ok(())
    }

    /// Subscribe trực tiếp vào broadcast.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<RunEvent> {
        self.inner.events.subscribe()
    }

    /// Nhận event; `Lagged` được log rồi tiếp tục, không làm run chết.
    pub async fn recv_event(
        &self,
        receiver: &mut broadcast::Receiver<RunEvent>,
    ) -> Option<RunEvent> {
        loop {
            match receiver.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "subscriber Router bị lag; tiếp tục stream");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Dừng worker outbox và huỷ các run đang active.
    ///
    /// Đây là shutdown của process, khác với `cancel` của người dùng: mọi token
    /// active đều được huỷ để tool/confirm không giữ tiến trình sống sau SIGTERM.
    pub fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let tokens = lock(&self.inner.state)
            .map(|state| {
                let mut tokens: Vec<_> = state
                    .active
                    .values()
                    .map(|active| active.cancel.clone())
                    .collect();
                tokens.extend(
                    state
                        .queues
                        .values()
                        .flat_map(|queue| queue.pending.iter().map(|job| job.cancel.clone())),
                );
                tokens
            })
            .unwrap_or_default();
        for token in tokens {
            token.cancel();
        }
    }

    /// Submit trả `RunId` ngay; run LLM được spawn nền.
    pub async fn submit(&self, incoming: Incoming) -> Result<RunId, RouterError> {
        self.authorize(&incoming)?;
        // (M21.1) Validate project trước khi xếp hàng — lỗi trả về cho adapter, không phải
        // giữa run.
        {
            let config = read_lock(&self.inner.config)?;
            self.resolve_project(&incoming, &config)?;
        }
        let _submission = self.inner.submission.lock().await;
        let run_id = new_run_id()?;
        let session = self.resolve_session(&incoming).await?;
        let text = incoming.text.trim();
        if text.starts_with('/') {
            self.handle_command(&incoming, session, run_id.clone(), text)
                .await?;
            return Ok(run_id);
        }

        self.enqueue(QueuedRun {
            run_id: run_id.clone(),
            session_id: session,
            incoming,
            cancel: CancellationToken::new(),
            background_allowed_tools: None,
        })?;
        Ok(run_id)
    }

    /// Submit một prompt tự động cho session đã được scheduler xác nhận.
    ///
    /// Khác với run interactive, mọi tool Confirm/Dangerous sẽ được RouterIo
    /// cho chạy nếu nằm trong `allowed_tools`, nếu không sẽ bị từ chối ngay.
    pub async fn submit_scheduled(
        &self,
        incoming: Incoming,
        allowed_tools: Vec<String>,
    ) -> Result<RunId, RouterError> {
        self.authorize(&incoming)?;
        {
            let config = read_lock(&self.inner.config)?;
            self.resolve_project(&incoming, &config)?;
        }
        let _submission = self.inner.submission.lock().await;
        let run_id = new_run_id()?;
        let session = self.resolve_session(&incoming).await?;
        self.enqueue(QueuedRun {
            run_id: run_id.clone(),
            session_id: session,
            incoming,
            cancel: CancellationToken::new(),
            background_allowed_tools: Some(Arc::new(allowed_tools.into_iter().collect())),
        })?;
        Ok(run_id)
    }

    fn authorize(&self, incoming: &Incoming) -> Result<(), RouterError> {
        let config = read_lock(&self.inner.config)?;
        if !config
            .agent
            .allowed_users
            .iter()
            .any(|allowed| allowed == &incoming.user_id)
        {
            tracing::warn!(user_id = %incoming.user_id, "từ chối user ngoài allowlist");
            return Err(RouterError::Forbidden(incoming.user_id.clone()));
        }
        let prefix = format!("{}:", incoming.channel);
        if !incoming.user_id.starts_with(&prefix) {
            return Err(RouterError::InvalidIdentity {
                channel: incoming.channel.clone(),
            });
        }
        Ok(())
    }

    /// Chọn project profile cho lượt (M21.1).
    ///
    /// Tên lạ ⇒ `RouterError::InvalidProject` (fail-closed: không rơi về project khác, vì
    /// làm vậy sẽ cho phiên đọc/ghi nhầm `MEMORY.md` của project khác).
    fn resolve_project(&self, incoming: &Incoming, config: &Config) -> Result<String, RouterError> {
        let requested = incoming
            .project
            .as_deref()
            .unwrap_or(beanagent_types::config::DEFAULT_PROJECT);
        if config.project_workspace(requested).is_some() {
            Ok(requested.to_string())
        } else {
            Err(RouterError::InvalidProject(requested.to_string()))
        }
    }

    async fn resolve_session(&self, incoming: &Incoming) -> Result<SessionId, RouterError> {
        if let Some(session) = incoming.session_id {
            let info = self.inner.store.session_info(session).await?;
            let owned = info.is_some_and(|info| {
                !info.archived
                    && info.channel == incoming.channel
                    && info.chat_id == incoming.chat_id
                    && info.user_id == incoming.user_id
            });
            if !owned {
                return Err(RouterError::InvalidSession);
            }
            return Ok(session);
        }
        self.inner
            .store
            .ensure_session_for_user(&incoming.channel, &incoming.chat_id, &incoming.user_id, "")
            .await
            .map_err(RouterError::Store)
    }
}

impl Router {
    fn enqueue(&self, queued: QueuedRun) -> Result<(), RouterError> {
        let (start, position) = {
            let mut state = lock(&self.inner.state)?;
            let queue = state
                .queues
                .entry(queued.session_id)
                .or_insert_with(|| SessionQueue {
                    active: None,
                    pending: VecDeque::new(),
                });
            if queue.active.is_some() {
                queue.pending.push_back(queued.clone());
                (false, Some(queue.pending.len() as u32))
            } else {
                queue.active = Some(queued.run_id.clone());
                state
                    .policies
                    .entry(queued.session_id)
                    .or_insert_with(|| Arc::new(SessionPolicy::new()));
                state.active.insert(
                    queued.run_id.clone(),
                    ActiveRun {
                        channel: queued.incoming.channel.clone(),
                        chat_id: queued.incoming.chat_id.clone(),
                        cancel: queued.cancel.clone(),
                    },
                );
                (true, None)
            }
        };
        if let Some(position) = position {
            self.emit(RunEvent::Queued {
                session_id: queued.session_id,
                run_id: queued.run_id.clone(),
                position,
            });
        }
        if start {
            self.spawn(queued);
        }
        Ok(())
    }

    fn spawn(&self, queued: QueuedRun) {
        let router = self.clone();
        tokio::spawn(async move {
            router.execute(queued.clone()).await;
            router.finish(&queued.run_id, queued.session_id);
        });
    }

    async fn execute(&self, queued: QueuedRun) {
        let config = match read_lock(&self.inner.config) {
            Ok(config) => config.clone(),
            Err(error) => {
                self.emit_error(&queued, "router_state", &error.to_string());
                return;
            }
        };
        let policy = if queued.background_allowed_tools.is_some() {
            // Scheduler không dùng allow-in-session của một phiên tương tác.
            Arc::new(SessionPolicy::new())
        } else {
            lock(&self.inner.state)
                .ok()
                .and_then(|state| state.policies.get(&queued.session_id).cloned())
                .unwrap_or_else(|| Arc::new(SessionPolicy::new()))
        };
        let audit_channel = if queued.background_allowed_tools.is_some() {
            "scheduler"
        } else {
            &queued.incoming.channel
        };
        let io = Arc::new(RouterIo {
            inner: Arc::downgrade(&self.inner),
            session_id: queued.session_id,
            run_id: queued.run_id.clone(),
            user_id: queued.incoming.user_id.clone(),
            cancel: queued.cancel.clone(),
            background_allowed_tools: queued.background_allowed_tools.clone(),
            last_actor: Mutex::new(None),
        });
        let skills_index = match read_lock(&self.inner.skills_index) {
            Ok(index) => index.clone(),
            Err(error) => {
                self.emit_error(&queued, "router_state", &error.to_string());
                return;
            }
        };
        // (M21.3) **Điểm quyết định RBAC duy nhất.** Router resolve role → `RolePermissions`
        // một lần mỗi run rồi truyền struct tuần tự hoá được xuống agent loop. Mọi lần lọc
        // tool (payload và tầng thực thi) đều dùng **cùng** struct này nên không thể lệch nhau,
        // và không logic RBAC nào nằm rải trong tool/role (ràng buộc `Plan.md` mục 4.3).
        let permissions: RolePermissions = config.permissions_for(&queued.incoming.user_id);
        // (M21.1) Project của lượt; `execute` chỉ chạy sau khi `submit` đã validate qua
        // `resolve_project`, nên ở đây dùng `unwrap_or(default)` là lưới an toàn.
        let project = queued
            .incoming
            .project
            .clone()
            .unwrap_or_else(|| beanagent_types::config::DEFAULT_PROJECT.to_string());
        tracing::debug!(
            run_id = %queued.run_id,
            user_id = %queued.incoming.user_id,
            role = %permissions.role,
            project = %project,
            "resolve quyền cho run"
        );
        let result = run_turn_outcome(RunTurnArgs {
            store: self.inner.store.as_ref(),
            registry: self.inner.registry.as_ref(),
            llm: self.inner.llm.as_ref(),
            config: &config,
            session: queued.session_id,
            user_text: queued.incoming.text.clone(),
            io,
            cancel: queued.cancel.clone(),
            session_policy: Some(policy),
            audit: self.inner.audit.clone(),
            channel: audit_channel,
            skills_index: &skills_index,
            permissions: &permissions,
            project: &project,
            alerts: self.alert_sink(),
        })
        .await;
        match result {
            Ok(outcome) => {
                let should_reflect = outcome.ended == crate::agent::EndReason::Final
                    && outcome.tool_call_count
                        >= usize::try_from(config.learning.min_tool_calls).unwrap_or(usize::MAX);
                self.emit(RunEvent::Final {
                    session_id: queued.session_id,
                    run_id: queued.run_id.clone(),
                    text: outcome.text.clone(),
                    message_id: outcome.message_id,
                });
                if should_reflect && let Some(reservation) = self.reserve_learning(&config) {
                    self.spawn_reflection(queued, config, outcome, reservation);
                }
            }
            Err(error) => {
                let code = agent_error_code(&error);
                self.emit_error(&queued, code, &error.to_string());
            }
        }
    }

    fn reserve_learning(&self, config: &Config) -> Option<chrono::DateTime<chrono::Utc>> {
        if !config.learning.enabled {
            return None;
        }
        let catalog = self.inner.skills.as_ref()?;
        let now = chrono::Utc::now();
        let last_proposal = match catalog.last_proposal_at() {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(error = %error, "state learning không hợp lệ; bỏ qua reflection");
                return None;
            }
        };
        if let Some(last) = last_proposal {
            match chrono::DateTime::parse_from_rfc3339(&last) {
                Ok(last) => {
                    let interval = chrono::Duration::from_std(std::time::Duration::from_secs(
                        config.learning.proposal_interval_minutes.saturating_mul(60),
                    ))
                    .unwrap_or(chrono::Duration::MAX);
                    if now < last.with_timezone(&chrono::Utc) + interval {
                        return None;
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "state learning có timestamp không hợp lệ");
                    return None;
                }
            }
        }
        let mut gate = lock(&self.inner.learning_gate).ok()?;
        let interval = chrono::Duration::from_std(std::time::Duration::from_secs(
            config.learning.proposal_interval_minutes.saturating_mul(60),
        ))
        .unwrap_or(chrono::Duration::MAX);
        if gate.is_some_and(|last| now < last + interval) {
            return None;
        }
        *gate = Some(now);
        Some(now)
    }

    fn release_learning(&self, reservation: chrono::DateTime<chrono::Utc>) {
        if let Ok(mut gate) = self.inner.learning_gate.lock()
            && *gate == Some(reservation)
        {
            *gate = None;
        }
    }

    fn spawn_reflection(
        &self,
        queued: QueuedRun,
        config: Config,
        outcome: RunOutcome,
        reservation: chrono::DateTime<chrono::Utc>,
    ) {
        let Some(catalog) = self.inner.skills.clone() else {
            self.release_learning(reservation);
            return;
        };
        let router = self.clone();
        let shutdown = self.inner.shutdown.clone();
        let created_at = reservation.to_rfc3339();
        tokio::spawn(async move {
            let result = reflect(ReflectionArgs {
                store: router.inner.store.as_ref(),
                llm: router.inner.llm.as_ref(),
                config: &config,
                catalog: &catalog,
                transcript: &outcome.transcript,
                loaded_skills: &outcome.loaded_skills,
                session: queued.session_id,
                channel: &queued.incoming.channel,
                chat_id: &queued.incoming.chat_id,
                created_at: &created_at,
                cancel: shutdown,
            })
            .await;
            match result {
                Ok(Some(draft)) => {
                    let verb = match draft.kind {
                        beanagent_skills::SkillDraftKind::New => "mới",
                        beanagent_skills::SkillDraftKind::Update => "sửa",
                    };
                    let text = format!(
                        "Đề xuất skill {verb} `{}` đã sẵn sàng để duyệt.\nLý do: {}\nDuyệt: /approve {} · Bỏ: /reject {}",
                        draft.name, draft.reason, draft.id, draft.id
                    );
                    match router
                        .inner
                        .store
                        .append(
                            queued.session_id,
                            beanagent_types::Message::assistant(Some(text.clone()), Vec::new()),
                        )
                        .await
                    {
                        Ok(message_id) => {
                            let outbound = beanagent_types::Outbound {
                                session_id: queued.session_id,
                                message_id,
                                text,
                                kind: beanagent_types::OutboundKind::Notification,
                                action: Some(beanagent_types::OutboundAction::SkillDraft {
                                    id: draft.id,
                                    name: draft.name,
                                    actor: queued.incoming.user_id.clone(),
                                }),
                            };
                            if let Err(error) = router
                                .notify(
                                    &queued.incoming.channel,
                                    &queued.incoming.chat_id,
                                    outbound,
                                )
                                .await
                            {
                                tracing::warn!(error = %error, "gửi thông báo skill nháp thất bại");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "không lưu thông báo skill nháp");
                        }
                    }
                }
                Ok(None) => router.release_learning(reservation),
                Err(error) => {
                    tracing::warn!(error = %error, "reflection M15 thất bại; run chính vẫn hoàn tất");
                    router.release_learning(reservation);
                }
            }
        });
    }

    fn finish(&self, run_id: &RunId, session_id: SessionId) {
        let (next, positions, stale) = {
            let mut state = match lock(&self.inner.state) {
                Ok(state) => state,
                Err(_) => return,
            };
            state.active.remove(run_id);
            let stale: Vec<_> = state
                .confirms
                .iter()
                .filter(|(_, pending)| pending.run_id == *run_id)
                .map(|(id, pending)| (id.clone(), pending.session_id, pending.run_id.clone()))
                .collect();
            for (id, _, _) in &stale {
                state.confirms.remove(id);
            }
            let Some(queue) = state.queues.get_mut(&session_id) else {
                return;
            };
            if queue.active.as_ref() != Some(run_id) {
                return;
            }
            queue.active = None;
            let shutting_down = self.inner.shutdown.is_cancelled();
            let next = if shutting_down {
                queue.pending.clear();
                None
            } else {
                queue.pending.pop_front()
            };
            let positions: Vec<_> = queue
                .pending
                .iter()
                .enumerate()
                .map(|(index, job)| (job.run_id.clone(), index as u32 + 1))
                .collect();
            if let Some(next) = &next {
                queue.active = Some(next.run_id.clone());
                state.active.insert(
                    next.run_id.clone(),
                    ActiveRun {
                        channel: next.incoming.channel.clone(),
                        chat_id: next.incoming.chat_id.clone(),
                        cancel: next.cancel.clone(),
                    },
                );
            }
            (next, positions, stale)
        };
        for (confirm_id, confirm_session, confirm_run) in stale {
            self.emit(RunEvent::ConfirmResolved {
                session_id: confirm_session,
                run_id: confirm_run,
                confirm_id,
                outcome: ConfirmOutcome::Denied,
            });
        }
        for (run_id, position) in positions {
            self.emit(RunEvent::Queued {
                session_id,
                run_id,
                position,
            });
        }
        if let Some(next) = next {
            self.spawn(next);
        }
    }
}

impl Router {
    /// Đọc snapshot run/confirm hiện tại cho WebSocket `Sync`.
    pub fn snapshot(&self) -> RouterSnapshot {
        let state = match lock(&self.inner.state) {
            Ok(state) => state,
            Err(_) => {
                return RouterSnapshot {
                    running: Vec::new(),
                    pending_confirms: Vec::new(),
                };
            }
        };
        let running = state
            .active
            .keys()
            .map(|run_id| RunningInfo {
                run_id: run_id.clone(),
                session_id: state
                    .queues
                    .iter()
                    .find_map(|(session, queue)| {
                        (queue.active.as_ref() == Some(run_id)).then_some(*session)
                    })
                    .unwrap_or_else(|| SessionId::new(0)),
            })
            .collect();
        let pending_confirms = state
            .confirms
            .iter()
            .map(|(id, pending)| PendingConfirmInfo {
                confirm_id: id.clone(),
                session_id: pending.session_id,
                run_id: pending.run_id.clone(),
                prompt: pending.prompt.clone(),
                risk: pending.risk,
                allow_session_option: pending.allow_session_option,
                timeout_seconds: pending.timeout_seconds,
            })
            .collect();
        RouterSnapshot {
            running,
            pending_confirms,
        }
    }

    /// Huỷ active run của channel/chat; queued run không bị huỷ.
    pub async fn cancel(&self, channel: &str, chat_id: &str) {
        let token = lock(&self.inner.state).ok().and_then(|state| {
            state
                .active
                .iter()
                .find(|(_, active)| active.channel == channel && active.chat_id == chat_id)
                .map(|(_, active)| active.cancel.clone())
        });
        if let Some(token) = token {
            token.cancel();
        }
    }

    /// Huỷ đúng run đang active của một session.
    ///
    /// WebSocket gửi `session_id`, nên adapter không được dùng `cancel(channel,
    /// chat_id)` vì nhiều session của cùng user có thể bị huỷ nhầm.
    pub async fn cancel_session(&self, session: SessionId) {
        let run_id = lock(&self.inner.state).ok().and_then(|state| {
            state
                .queues
                .get(&session)
                .and_then(|queue| queue.active.clone())
        });
        let token = run_id.and_then(|run_id| {
            lock(&self.inner.state).ok().and_then(|state| {
                state
                    .active
                    .get(&run_id)
                    .map(|active| active.cancel.clone())
            })
        });
        if let Some(token) = token {
            token.cancel();
        }
    }

    /// Active run hiện tại, dùng cho test/Sync M9.
    #[must_use]
    pub fn active_run(&self, channel: &str, chat_id: &str) -> Option<RunId> {
        let state = lock(&self.inner.state).ok()?;
        state
            .active
            .iter()
            .find(|(_, active)| active.channel == channel && active.chat_id == chat_id)
            .map(|(run_id, _)| run_id.clone())
    }

    fn session_busy(&self, session: SessionId) -> bool {
        lock(&self.inner.state).is_ok_and(|state| {
            state
                .queues
                .get(&session)
                .is_some_and(|queue| queue.active.is_some() || !queue.pending.is_empty())
        })
    }

    async fn handle_command(
        &self,
        incoming: &Incoming,
        session: SessionId,
        run_id: RunId,
        command: &str,
    ) -> Result<(), RouterError> {
        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or_default();
        let argument = parts.next().unwrap_or_default();
        if parts.next().is_some() {
            return self.command_error(session, run_id, "invalid_arguments", "Quá nhiều tham số.");
        }
        let result: Result<String, RouterError> = match name {
            "/new" => {
                if self.session_busy(session) {
                    Err(RouterError::SessionBusy)
                } else {
                    self.inner.store.archive_session(session).await?;
                    lock(&self.inner.state)?.policies.remove(&session);
                    let new_session = self
                        .inner
                        .store
                        .ensure_session_for_user(
                            &incoming.channel,
                            &incoming.chat_id,
                            &incoming.user_id,
                            "",
                        )
                        .await?;
                    Ok(format!("Đã tạo phiên mới: {new_session}"))
                }
            }
            "/stop" => {
                self.cancel_session(session).await;
                Ok("Đã yêu cầu dừng run hiện tại.".into())
            }
            "/model" => {
                if argument.is_empty() {
                    let config = read_lock(&self.inner.config)?;
                    Ok(format!(
                        "Model hiện tại: {}. Cho phép: {}",
                        config.llm.model,
                        config.llm.effective_allowed_models().join(", ")
                    ))
                } else {
                    let mut config = write_lock(&self.inner.config)?;
                    if config
                        .llm
                        .effective_allowed_models()
                        .iter()
                        .any(|model| model == argument)
                    {
                        config.llm.model = argument.to_string();
                        Ok(format!("Model đổi thành: {argument}"))
                    } else {
                        Err(RouterError::ModelNotAllowed(argument.to_string()))
                    }
                }
            }
            "/skills" => {
                let index = read_lock(&self.inner.skills_index)?.clone();
                Ok(if index.trim().is_empty() {
                    "Chưa nạp skill.".into()
                } else {
                    index
                })
            }
            "/memory" => {
                if argument.is_empty() {
                    Ok("Dùng: /memory <truy vấn>".into())
                } else {
                    let hits = self.inner.store.memory_search(argument).await?;
                    Ok(if hits.is_empty() {
                        "Không tìm thấy ghi nhớ.".into()
                    } else {
                        hits.into_iter()
                            .map(|hit| format!("- {}", hit.text))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                }
            }
            "/tasks" => Ok("Chưa có tác vụ định kỳ.".into()),
            "/approve" if argument.is_empty() => Ok("Dùng: /approve <id>".into()),
            "/reject" if argument.is_empty() => Ok("Dùng: /reject <id>".into()),
            "/approve" => self
                .approve_draft(argument, &incoming.user_id)
                .await
                .map(|decision| format!("Đã duyệt và kích hoạt skill `{}`.", decision.name)),
            "/reject" => self
                .reject_draft(argument, &incoming.user_id)
                .await
                .map(|decision| format!("Đã bỏ skill nháp `{}`.", decision.name)),
            other => Err(RouterError::UnknownCommand(other.to_string())),
        };
        match result {
            Ok(text) => {
                self.emit(RunEvent::Final {
                    session_id: session,
                    run_id,
                    text,
                    message_id: None,
                });
                Ok(())
            }
            Err(error) => {
                self.emit(RunEvent::Error {
                    session_id: session,
                    run_id,
                    code: router_error_code(&error).into(),
                    message: error.to_string(),
                });
                Ok(())
            }
        }
    }

    fn command_error(
        &self,
        session: SessionId,
        run_id: RunId,
        code: &str,
        message: &str,
    ) -> Result<(), RouterError> {
        self.emit(RunEvent::Error {
            session_id: session,
            run_id,
            code: code.into(),
            message: message.into(),
        });
        Ok(())
    }
}

impl Router {
    /// Duyệt skill nháp sau khi actor đã xác thực ở lớp channel.
    pub async fn approve_draft(
        &self,
        id: &str,
        actor: &str,
    ) -> Result<SkillDraftDecision, RouterError> {
        self.authorize_actor(actor)?;
        let catalog = self.skills_catalog()?;
        let draft_id = id.to_string();
        let decision = tokio::task::spawn_blocking(move || catalog.approve_draft(&draft_id))
            .await
            .map_err(|error| RouterError::Internal(error.to_string()))??;
        self.refresh_skills_index()?;
        Ok(decision)
    }

    /// Bỏ skill nháp; không reload vì không có skill nào được kích hoạt.
    pub async fn reject_draft(
        &self,
        id: &str,
        actor: &str,
    ) -> Result<SkillDraftDecision, RouterError> {
        self.authorize_actor(actor)?;
        let catalog = self.skills_catalog()?;
        let draft_id = id.to_string();
        tokio::task::spawn_blocking(move || catalog.reject_draft(&draft_id))
            .await
            .map_err(|error| RouterError::Internal(error.to_string()))?
            .map_err(Into::into)
    }

    fn skills_catalog(&self) -> Result<SkillCatalog, RouterError> {
        self.inner
            .skills
            .clone()
            .ok_or_else(|| SkillError::NotFound("skill catalog".into()).into())
    }

    fn authorize_actor(&self, actor: &str) -> Result<(), RouterError> {
        let config = read_lock(&self.inner.config)?;
        if config
            .agent
            .allowed_users
            .iter()
            .any(|allowed| allowed == actor)
        {
            Ok(())
        } else {
            Err(RouterError::Forbidden(actor.to_string()))
        }
    }

    fn refresh_skills_index(&self) -> Result<(), RouterError> {
        let catalog = self.skills_catalog()?;
        *write_lock(&self.inner.skills_index)? = catalog.index();
        Ok(())
    }

    /// Phản hồi hợp lệ đầu tiên thắng; response sau trả `ConfirmNotFound`.
    pub async fn resolve_confirm(
        &self,
        confirm_id: &str,
        decision: Decision,
        actor: &str,
    ) -> Result<(), RouterError> {
        let id = ConfirmId::new(confirm_id);
        let (session_id, run_id, sender) = {
            let mut state = lock(&self.inner.state)?;
            let pending = state
                .confirms
                .remove(&id)
                .ok_or(RouterError::ConfirmNotFound)?;
            if pending.actor != actor {
                state.confirms.insert(id.clone(), pending);
                return Err(RouterError::ConfirmForbidden);
            }
            if matches!(decision, Decision::AllowInSession) && !pending.allow_session_option {
                state.confirms.insert(id.clone(), pending);
                return Err(RouterError::ConfirmForbidden);
            }
            (pending.session_id, pending.run_id, pending.sender)
        };
        sender
            .send(Confirmation {
                decision,
                actor: actor.to_string(),
            })
            .map_err(|_| RouterError::ConfirmNotFound)?;
        self.emit(RunEvent::ConfirmResolved {
            session_id,
            run_id,
            confirm_id: id,
            outcome: match decision {
                Decision::Allow | Decision::AllowInSession => ConfirmOutcome::Allowed,
                Decision::Deny => ConfirmOutcome::Denied,
            },
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_confirm(
        &self,
        session_id: SessionId,
        run_id: &RunId,
        risk: Risk,
        prompt: &str,
        allow_session: bool,
        actor: &str,
        requested_timeout: Duration,
    ) -> Result<(ConfirmId, oneshot::Receiver<Confirmation>), RouterError> {
        let confirm_id = new_confirm_id()?;
        let (sender, receiver) = oneshot::channel();
        let timeout = requested_timeout.min(self.inner.options.confirm_timeout);
        let timeout_seconds = u32::try_from(timeout.as_secs()).unwrap_or(u32::MAX);
        lock(&self.inner.state)?.confirms.insert(
            confirm_id.clone(),
            PendingConfirm {
                run_id: run_id.clone(),
                session_id,
                actor: actor.to_string(),
                prompt: prompt.to_string(),
                risk,
                allow_session_option: allow_session,
                timeout_seconds,
                sender,
            },
        );
        self.emit(RunEvent::ConfirmRequest {
            session_id,
            run_id: run_id.clone(),
            confirm_id: confirm_id.clone(),
            prompt: prompt.to_string(),
            risk,
            allow_session_option: allow_session,
            timeout_seconds,
        });
        Ok((confirm_id, receiver))
    }

    fn expire_confirm(&self, confirm_id: &ConfirmId, outcome: ConfirmOutcome) -> bool {
        let removed = lock(&self.inner.state)
            .ok()
            .and_then(|mut state| state.confirms.remove(confirm_id));
        let Some(pending) = removed else {
            return false;
        };
        self.emit(RunEvent::ConfirmResolved {
            session_id: pending.session_id,
            run_id: pending.run_id,
            confirm_id: confirm_id.clone(),
            outcome,
        });
        true
    }
}

impl Router {
    /// Gửi outbound; lỗi/channel chưa đăng ký sẽ vào outbox.
    pub async fn notify(
        &self,
        channel: &str,
        chat_id: &str,
        out: Outbound,
    ) -> Result<(), RouterError> {
        let adapter = read_lock(&self.inner.channels)?.get(channel).cloned();
        let mut delivered = false;
        if let Some(adapter) = adapter {
            match adapter.send(chat_id, out.clone()).await {
                Ok(()) => delivered = true,
                Err(error) => {
                    tracing::warn!(channel, error = %error, "gửi outbound lỗi; lưu outbox");
                }
            }
        }
        if !delivered {
            self.inner
                .store
                .enqueue_outbound(channel, chat_id, &out, &now_rfc3339())
                .await?;
        }
        Ok(())
    }

    /// Dựng [`AlertSink`] gửi cảnh báo M23 tới kênh chính, hoặc `None` nếu chưa cấu hình.
    ///
    /// Trả `None` khi `alert_channel`/`alert_chat_id` trống — đó là cấu hình "không gửi
    /// cảnh báo trực tiếp", và `Config::validate` đã chặn trạng thái bật nửa chừng.
    #[must_use]
    pub fn alert_sink(&self) -> Option<Arc<dyn AlertSink>> {
        let config = read_lock(&self.inner.config).ok()?;
        let channel = config.security_scan.alert_channel.trim();
        let chat_id = config.security_scan.alert_chat_id.trim();
        if channel.is_empty() || chat_id.is_empty() {
            return None;
        }
        Some(Arc::new(RouterAlertSink {
            router: Arc::new(self.clone()),
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
        }))
    }

    /// Khởi động worker retry outbox đúng một lần.
    pub fn start_outbox_worker(self: &Arc<Self>) -> Result<(), RouterError> {
        if self
            .inner
            .outbox_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        let handle = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                self.inner.outbox_started.store(false, Ordering::Release);
                return Err(RouterError::NoRuntime);
            }
        };
        let weak = Arc::downgrade(&self.inner);
        let shutdown = self.inner.shutdown.clone();
        let interval = self.inner.options.outbox_poll_interval;
        handle.spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {
                        let Some(inner) = weak.upgrade() else { break };
                        let router = Self { inner };
                        if let Err(error) = router.process_outbox_once().await {
                            tracing::warn!(error = %error, "worker outbox lỗi");
                        }
                    }
                }
            }
        });
        Ok(())
    }

    /// Xử lý một batch outbox đến hạn (public để test không phụ thuộc wall clock).
    pub async fn process_outbox_once(&self) -> Result<usize, RouterError> {
        let entries = self
            .inner
            .store
            .due_outbox(&now_rfc3339(), OUTBOX_BATCH_SIZE)
            .await?;
        let count = entries.len();
        for entry in entries {
            let adapter = read_lock(&self.inner.channels)?
                .get(&entry.channel)
                .cloned();
            let result = match adapter {
                Some(adapter) => adapter.send(&entry.chat_id, entry.payload.clone()).await,
                None => Err(anyhow::anyhow!("channel chưa đăng ký: {}", entry.channel)),
            };
            match result {
                Ok(()) => self.inner.store.complete_outbox(entry.id).await?,
                Err(error) => {
                    let next_attempt_at = self.next_outbox_attempt(entry.attempts);
                    self.inner
                        .store
                        .retry_outbox(entry.id, &next_attempt_at, &error.to_string())
                        .await?;
                }
            }
        }
        Ok(count)
    }

    fn next_outbox_attempt(&self, attempts: u32) -> String {
        let factor = 1_u32.checked_shl(attempts.min(16)).unwrap_or(u32::MAX);
        let delay = self
            .inner
            .options
            .outbox_base_delay
            .saturating_mul(factor)
            .min(self.inner.options.outbox_max_delay);
        let delay = chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::MAX);
        (chrono::Utc::now() + delay).to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
    }
}

struct RouterIo {
    inner: Weak<RouterInner>,
    session_id: SessionId,
    run_id: RunId,
    user_id: String,
    cancel: CancellationToken,
    background_allowed_tools: Option<Arc<HashSet<String>>>,
    last_actor: Mutex<Option<String>>,
}

#[async_trait::async_trait]
impl RunIo for RouterIo {
    fn on_text(&self, text: &str) {
        self.with_router(|router| {
            router.emit(RunEvent::Text {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                text: text.to_string(),
            });
        });
    }

    fn on_text_delta(&self, text: &str, index: u32, reset: bool) {
        self.with_router(|router| {
            router.emit(RunEvent::TextDelta {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                text: text.to_string(),
                index,
                reset,
            });
        });
    }

    fn on_tool_start(&self, id: &str, tool: &str, risk: Risk, summary: &str, args: &str) {
        self.with_router(|router| {
            router.emit(RunEvent::ToolStart {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                id: id.to_string(),
                tool: tool.to_string(),
                summary: preview(summary),
                args_preview: preview(args),
                risk,
            });
        });
    }

    fn on_tool_end(&self, id: &str, tool: &str, ok: bool, output: &str) {
        self.with_router(|router| {
            router.emit(RunEvent::ToolEnd {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                id: id.to_string(),
                tool: tool.to_string(),
                ok,
                output_preview: preview(output),
            });
        });
    }

    async fn confirm(
        &self,
        _id: &str,
        _tool: &str,
        risk: Risk,
        prompt: &str,
        allow_in_session: bool,
        timeout: Duration,
    ) -> Option<Decision> {
        if self.background_allowed_tools.is_some() {
            return Some(if self.background_tool_allowed(_tool) {
                Decision::Allow
            } else {
                Decision::Deny
            });
        }
        let inner = self.inner.upgrade()?;
        let router = Router { inner };
        let (confirm_id, receiver) = router
            .begin_confirm(
                self.session_id,
                &self.run_id,
                risk,
                prompt,
                allow_in_session,
                &self.user_id,
                timeout,
            )
            .ok()?;
        let effective_timeout = timeout.min(router.inner.options.confirm_timeout);
        let mut receiver = receiver;
        let decision = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => {
                if router.expire_confirm(&confirm_id, ConfirmOutcome::Denied) {
                    None
                } else {
                    receiver.await.ok().map(|confirmation| {
                        remember_actor(&self.last_actor, confirmation.actor);
                        confirmation.decision
                    })
                }
            }
            response = &mut receiver => response.ok().map(|confirmation| {
                remember_actor(&self.last_actor, confirmation.actor);
                confirmation.decision
            }),
            _ = tokio::time::sleep(effective_timeout) => {
                if router.expire_confirm(&confirm_id, ConfirmOutcome::Expired) {
                    None
                } else {
                    receiver.await.ok().map(|confirmation| {
                        remember_actor(&self.last_actor, confirmation.actor);
                        confirmation.decision
                    })
                }
            }
        };
        decision
    }

    fn decision_actor(&self) -> Option<String> {
        if self.background_allowed_tools.is_some() {
            return Some("scheduler".into());
        }
        self.last_actor.lock().ok().and_then(|actor| actor.clone())
    }

    fn is_background(&self) -> bool {
        self.background_allowed_tools.is_some()
    }

    fn background_tool_allowed(&self, tool: &str) -> bool {
        self.background_allowed_tools
            .as_ref()
            .is_some_and(|tools| tools.contains(tool))
    }

    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

impl RouterIo {
    fn with_router(&self, action: impl FnOnce(&Router)) {
        if let Some(inner) = self.inner.upgrade() {
            action(&Router { inner });
        }
    }
}

impl Router {
    fn emit(&self, event: RunEvent) {
        let _ = self.inner.events.send(event);
    }

    fn emit_error(&self, queued: &QueuedRun, code: &str, message: &str) {
        self.emit(RunEvent::Error {
            session_id: queued.session_id,
            run_id: queued.run_id.clone(),
            code: code.into(),
            message: message.into(),
        });
    }
}

fn remember_actor(slot: &Mutex<Option<String>>, actor: String) {
    if let Ok(mut current) = slot.lock() {
        *current = Some(actor);
    }
}

fn preview(text: &str) -> String {
    match truncate_chars(text, PREVIEW_CHARS) {
        Some((kept, _)) => kept.to_string(),
        None => text.to_string(),
    }
}

fn new_run_id() -> Result<RunId, RouterError> {
    random_id("run_").map(RunId::new)
}

fn new_confirm_id() -> Result<ConfirmId, RouterError> {
    random_id("confirm_").map(ConfirmId::new)
}

fn random_id(prefix: &str) -> Result<String, RouterError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| RouterError::Random(error.to_string()))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(prefix.len() + 32);
    id.push_str(prefix);
    for byte in bytes {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(id)
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, RouterError> {
    mutex.lock().map_err(|_| RouterError::StatePoisoned)
}

fn read_lock<T>(lock: &RwLock<T>) -> Result<RwLockReadGuard<'_, T>, RouterError> {
    lock.read().map_err(|_| RouterError::StatePoisoned)
}

fn write_lock<T>(lock: &RwLock<T>) -> Result<RwLockWriteGuard<'_, T>, RouterError> {
    lock.write().map_err(|_| RouterError::StatePoisoned)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn agent_error_code(error: &AgentError) -> &'static str {
    match error {
        AgentError::Store(_) => "store",
        AgentError::Llm(_) => "llm",
        AgentError::Cancelled => "cancelled",
        AgentError::RepeatFailure(_) => "loop_guard",
    }
}

fn router_error_code(error: &RouterError) -> &'static str {
    match error {
        RouterError::Forbidden(_) | RouterError::InvalidIdentity { .. } => "forbidden",
        RouterError::InvalidProject(_) => "invalid_project",
        RouterError::Store(_) => "store",
        RouterError::Skill(_) => "skill_draft",
        RouterError::Internal(_) => "internal",
        RouterError::InvalidSession => "invalid_session",
        RouterError::SessionBusy => "session_busy",
        RouterError::UnknownCommand(_) => "unknown_command",
        RouterError::ModelNotAllowed(_) => "model_not_allowed",
        RouterError::ConfirmNotFound => "confirm_not_found",
        RouterError::ConfirmForbidden => "confirm_forbidden",
        RouterError::DuplicateChannel(_) => "duplicate_channel",
        RouterError::NoRuntime => "no_runtime",
        RouterError::StatePoisoned => "router_state",
        RouterError::Random(_) => "random_unavailable",
    }
}

/// Cài bản [`AlertSink`] bọc quanh [`Router::notify`] (M23).
///
/// Giữ **một** đường gửi: mọi cảnh báo đi qua `notify` nên lỗi vẫn rơi vào outbox và được
/// thử lại — không mất tin cảnh báo an ninh.
struct RouterAlertSink {
    router: Arc<Router>,
    channel: String,
    chat_id: String,
}

#[async_trait::async_trait]
impl AlertSink for RouterAlertSink {
    async fn send_alert(&self, alert: &Alert) -> Result<(), String> {
        let text = if alert.risks.is_empty() {
            format!("{}\n{}", alert.title, alert.summary)
        } else {
            format!(
                "{}\n{}\n- {}",
                alert.title,
                alert.summary,
                alert.risks.join("\n- ")
            )
        };
        // Ghi vào lịch sử trước để `message_id` có thật (adapter hiển thị và UI cần id này).
        let session = self
            .router
            .inner
            .store
            .ensure_session(&self.channel, &self.chat_id, &alert.title)
            .await
            .map_err(|error| error.to_string())?;
        let message_id = self
            .router
            .inner
            .store
            .append(
                session,
                beanagent_types::Message::assistant(Some(text.clone()), Vec::new()),
            )
            .await
            .map_err(|error| error.to_string())?;
        self.router
            .notify(
                &self.channel,
                &self.chat_id,
                Outbound {
                    session_id: session,
                    message_id,
                    text,
                    kind: OutboundKind::Notification,
                    action: None,
                },
            )
            .await
            .map_err(|error| error.to_string())
    }
}
