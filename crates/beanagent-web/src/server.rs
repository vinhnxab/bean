//! Axum application, REST, WebSocket và WebChannel cho M9.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Extension, Path, Query, Request, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, patch, post};
use axum::{Json, Router as AxumRouter};
use axum_extra::extract::cookie::CookieJar;
use beanagent_core::{Channel, Decision, Incoming, Router, next_run_after};
use beanagent_memory::{
    MemoryRecord, MessageRecord, NewScheduledTask, ScheduledTask, SessionSummary, Store,
};
use beanagent_security::AuditLog;
use beanagent_skills::{SkillCatalog, SkillDraft, SkillError};
use beanagent_tools::WorkspaceFs;
use beanagent_types::{Config, Outbound, Risk, RunEvent, SessionId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::api_types::*;
use crate::auth::{AuthError, AuthService};

/// Kích thước request HTTP tối đa.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Kích thước message WebSocket tối đa.
pub const MAX_WS_MESSAGE_BYTES: usize = 64 * 1024;
/// Nhịp heartbeat WebSocket.
pub const WS_HEARTBEAT: Duration = Duration::from_secs(30);
/// Đóng socket im lặng quá lâu.
pub const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Số kết nối WebSocket đồng thời tối đa cho một instance.
pub const MAX_WS_CONNECTIONS: usize = 32;

/// State runtime được chia sẻ bởi mọi request.
#[derive(Clone)]
pub struct WebState {
    /// Cấu hình đã validate.
    pub config: Config,
    /// Store dùng cho REST và Router.
    pub store: Arc<dyn Store>,
    /// Router sở hữu run, không gắn với kết nối.
    pub router: Arc<Router>,
    /// Auth service.
    pub auth: AuthService,
    /// Audit log tùy chọn.
    pub audit: Option<Arc<AuditLog>>,
    /// Catalog skill.
    pub skills: SkillCatalog,
    /// Workspace capability cho memory files.
    pub workspace: Option<Arc<dyn WorkspaceFs>>,
    /// Broadcast notification từ scheduler/channel.
    pub notifications: broadcast::Sender<ServerMsg>,
    /// Thời điểm process bắt đầu.
    pub started_at: Instant,
    /// Origin canonical đã validate.
    pub public_origin: Url,
    /// Giới hạn số WebSocket đồng thời; permit sống cùng connection.
    ws_connections: Arc<Semaphore>,
}

impl std::fmt::Debug for WebState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebState")
            .field("public_origin", &self.public_origin.as_str())
            .field("started_at", &self.started_at)
            .finish_non_exhaustive()
    }
}

impl WebState {
    /// Tạo state và kiểm tra `public_origin` không có path/query/userinfo.
    pub fn new(
        config: Config,
        store: Arc<dyn Store>,
        router: Arc<Router>,
        auth: AuthService,
        audit: Option<Arc<AuditLog>>,
        skills: SkillCatalog,
        workspace: Option<Arc<dyn WorkspaceFs>>,
    ) -> Result<Self, WebBuildError> {
        let public_origin =
            Url::parse(&config.web.public_origin).map_err(|_| WebBuildError::InvalidOrigin)?;
        if !matches!(public_origin.scheme(), "http" | "https")
            || public_origin.host_str().is_none()
            || !public_origin.username().is_empty()
            || public_origin.password().is_some()
            || public_origin.path() != "/"
            || public_origin.query().is_some()
            || public_origin.fragment().is_some()
        {
            return Err(WebBuildError::InvalidOrigin);
        }
        let (notifications, _) = broadcast::channel(256);
        Ok(Self {
            config,
            store,
            router,
            auth,
            audit,
            skills,
            workspace,
            notifications,
            started_at: Instant::now(),
            public_origin,
            ws_connections: Arc::new(Semaphore::new(MAX_WS_CONNECTIONS)),
        })
    }
}

/// Lỗi API đã lọc an toàn.
#[derive(Debug, Clone)]
pub struct ApiFailure {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiFailure {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "cần đăng nhập")
    }

    fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "origin hoặc quyền không hợp lệ",
        )
    }

    fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "không tìm thấy tài nguyên",
        )
    }

    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "lỗi nội bộ",
        )
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiError {
                code: self.code.into(),
                message: self.message,
            }),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<T, ApiFailure>;

/// Channel `web` chỉ gửi notification; run đã do Router sở hữu.
pub struct WebChannel {
    notifications: broadcast::Sender<ServerMsg>,
}

impl WebChannel {
    /// Tạo adapter từ broadcast channel của state.
    #[must_use]
    pub fn new(notifications: broadcast::Sender<ServerMsg>) -> Self {
        Self { notifications }
    }
}

#[async_trait::async_trait]
impl Channel for WebChannel {
    fn name(&self) -> &'static str {
        "web"
    }

    async fn run(&self, _router: Arc<Router>, shutdown: CancellationToken) -> anyhow::Result<()> {
        shutdown.cancelled().await;
        Ok(())
    }

    async fn send(&self, _chat_id: &str, out: Outbound) -> anyhow::Result<()> {
        let _ = self.notifications.send(ServerMsg::Notification {
            session_id: out.session_id.get(),
            message_id: out.message_id,
        });
        Ok(())
    }
}

/// Lỗi dựng app.
#[derive(Debug, thiserror::Error)]
pub enum WebBuildError {
    #[error("web.public_origin không hợp lệ")]
    InvalidOrigin,
}

/// Origin/Host/CSRF middleware cho mọi request thay đổi dữ liệu và WebSocket.
pub async fn request_guard(
    State(state): State<WebState>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let headers = request.headers().clone();
    if path.starts_with("/api/") && is_mutating(method) {
        if !host_matches(&headers, &state.public_origin)
            || !origin_matches(&headers, &state.public_origin)
        {
            return ApiFailure::forbidden().into_response();
        }
        if !has_json_content_type(&headers) {
            return ApiFailure::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "json_required",
                "Content-Type phải là application/json",
            )
            .into_response();
        }
    }
    if path == "/api/ws"
        && (!host_matches(&headers, &state.public_origin)
            || !origin_matches(&headers, &state.public_origin))
    {
        return ApiFailure::forbidden().into_response();
    }
    next.run(request).await
}

/// Thêm security headers sau khi handler xử lý xong.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; style-src 'self' 'unsafe-inline'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if path == "/api" || path.starts_with("/api/") {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

fn is_mutating(method: Method) -> bool {
    matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn host_matches(headers: &HeaderMap, origin: &Url) -> bool {
    let Some(host) = header_str(headers, header::HOST) else {
        return false;
    };
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    let expected = origin.host_str().unwrap_or_default();
    let expected_port = origin.port_or_known_default();
    let actual_port = authority.port_u16().or_else(|| {
        if origin.scheme() == "https" {
            Some(443)
        } else {
            Some(80)
        }
    });
    authority.host().eq_ignore_ascii_case(expected) && actual_port == expected_port
}

fn origin_matches(headers: &HeaderMap, origin: &Url) -> bool {
    header_str(headers, header::ORIGIN)
        .is_some_and(|value| value == origin.as_str().trim_end_matches('/'))
}

fn has_json_content_type(headers: &HeaderMap) -> bool {
    header_str(headers, header::CONTENT_TYPE).is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    })
}

/// Lấy token từ cookie, không log giá trị.
fn cookie_token(jar: &CookieJar) -> Option<String> {
    jar.get(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
}

/// Xác thực cookie cho REST handler.
async fn require_user(state: &WebState, jar: &CookieJar) -> ApiResult<String> {
    let token = cookie_token(jar).ok_or_else(ApiFailure::unauthorized)?;
    state
        .auth
        .authenticate_token(&token)
        .await
        .map(|info| info.user_id)
        .map_err(|_| ApiFailure::unauthorized())
}

/// Chuyển `Message` sang DTO JSON cho REST.
fn message_dto(record: MessageRecord) -> ApiResult<MessageDto> {
    let message = serde_json::to_value(record.message).map_err(|_| ApiFailure::internal())?;
    Ok(MessageDto {
        id: record.id,
        session_id: record.session_id.get(),
        seq: record.seq,
        message,
        created_at: record.created_at,
    })
}

fn session_dto(session: SessionSummary) -> SessionDto {
    SessionDto {
        id: session.id.get(),
        channel: session.channel,
        chat_id: session.chat_id,
        user_id: session.user_id,
        title: session.title,
        archived: session.archived,
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

fn memory_dto(memory: MemoryRecord) -> MemoryDto {
    MemoryDto {
        id: memory.id,
        text: memory.text,
        tags: memory.tags,
        created_at: memory.created_at,
    }
}

fn task_dto(task: ScheduledTask) -> TaskDto {
    TaskDto {
        id: task.id,
        session_id: task.session_id.map_or(0, |id| id.get()),
        cron: task.cron,
        prompt: task.prompt,
        channel: task.channel,
        chat_id: task.chat_id,
        allowed_tools: task.allowed_tools,
        next_run: task.next_run,
        enabled: task.enabled,
        created_at: task.created_at,
        last_run_at: task.last_run_at,
        last_status: task.last_status,
    }
}

/// Đăng nhập; không trả token trong JSON, chỉ đặt cookie.
async fn login(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiFailure> {
    let ip = peer.map_or(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        |Extension(ConnectInfo(peer))| peer.ip(),
    );
    let session = state
        .auth
        .login(&request.password, ip, state.audit.as_deref())
        .await
        .map_err(|error| match error {
            AuthError::InvalidCredentials => ApiFailure::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "thông tin đăng nhập không đúng",
            ),
            AuthError::RateLimited(seconds) => {
                let mut failure = ApiFailure::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "quá nhiều lần thử; thử lại sau",
                );
                failure.message = format!("quá nhiều lần thử; thử lại sau {seconds} giây");
                failure
            }
            _ => ApiFailure::internal(),
        })?;
    let mut response = Json(LoginResponse {
        user_id: state.config.web.user_id.clone(),
    })
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.auth.session_cookie(&session))
            .map_err(|_| ApiFailure::internal())?,
    );
    Ok(response)
}

/// Đăng xuất và xoá bản ghi server-side.
async fn logout(
    State(state): State<WebState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
) -> Result<Response, ApiFailure> {
    if let Some(token) = cookie_token(&jar) {
        let ip = peer.map_or(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            |Extension(ConnectInfo(peer))| peer.ip(),
        );
        state
            .auth
            .logout(&token, state.audit.as_deref(), ip)
            .await
            .map_err(|_| ApiFailure::internal())?;
    }
    let mut response = Json(LogoutResponse { ok: true }).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.auth.expired_cookie()).map_err(|_| ApiFailure::internal())?,
    );
    Ok(response)
}

async fn auth_me(State(state): State<WebState>, jar: CookieJar) -> ApiResult<Json<AuthMeResponse>> {
    let user_id = require_user(&state, &jar).await?;
    Ok(Json(AuthMeResponse { user_id }))
}

/// Trạng thái runtime cho API.
async fn status(State(state): State<WebState>, jar: CookieJar) -> ApiResult<Json<StatusResponse>> {
    let _ = require_user(&state, &jar).await?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let usage = state
        .store
        .usage(&today)
        .await
        .map_err(|_| ApiFailure::internal())?;
    let mut channels = Vec::with_capacity(2);
    if state.config.web.enabled {
        channels.push("web".to_string());
    }
    if state.config.telegram.enabled {
        channels.push("telegram".to_string());
    }
    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        model: state.config.llm.model.clone(),
        max_steps: state.config.agent.max_steps,
        daily_token_budget: state.config.security.daily_token_budget,
        tokens_used: u64::from(usage.total()),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        channels,
    }))
}

/// Danh sách agent cho HUB, **đã lọc theo RBAC trước khi tuần tự hoá**.
///
/// # Vì sao lọc ở đây chứ không ở client
///
/// Ràng buộc bắt buộc: role không có quyền phải **không nhận** dữ liệu đó trong
/// response, chứ không phải "UI tự ẩn đi". Ẩn bằng CSS/JS vẫn để nguyên payload
/// trong tab Network — đó là rò dữ liệu, không phải phân quyền.
///
/// Vì vậy `filter` chạy **trước** khi `AgentReportDto` được dựng: agent bị chặn
/// không bao giờ tồn tại trong response, và tên role của nó cũng không lộ ra.
///
/// Quyết định dùng [`RolePermissions::can_view_agent`] — cùng hệ tag RBAC, tách
/// khỏi `allows` vì đây là câu hỏi *quan sát*, không phải *quyền hành động*.
async fn list_agents(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<AgentListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let permissions = state.config.permissions_for(&user_id);
    let agents = state
        .router
        .agent_reports()
        .into_iter()
        .filter(|report| {
            state
                .config
                .role(&report.role)
                .is_some_and(|role| permissions.can_view_agent(&role.tag_set()))
        })
        .map(|report| AgentReportDto {
            role: report.role,
            status: match report.status {
                beanagent_types::AgentStatus::Idle => AgentStatusDto::Idle,
                beanagent_types::AgentStatus::Working => AgentStatusDto::Working,
                beanagent_types::AgentStatus::AwaitingYou => AgentStatusDto::AwaitingYou,
            },
            summary: report.summary,
            risks: report.risks,
            relation: match report.relation {
                beanagent_types::AgentRelation::Manages => AgentRelationDto::Manages,
                beanagent_types::AgentRelation::Reviews => AgentRelationDto::Reviews,
                beanagent_types::AgentRelation::AlertsDirectly => AgentRelationDto::AlertsDirectly,
            },
        })
        .collect();
    Ok(Json(AgentListResponse {
        agents,
        viewer_role: permissions.role,
    }))
}

async fn list_sessions(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<SessionQuery>,
) -> ApiResult<Json<SessionListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let sessions = state
        .store
        .list_sessions(
            &user_id,
            query.q.as_deref(),
            query.archived,
            query.limit.unwrap_or(100).min(500),
        )
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(SessionListResponse {
        sessions: sessions.into_iter().map(session_dto).collect(),
    }))
}

async fn create_session(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<CreateSessionRequest>,
) -> ApiResult<(StatusCode, Json<SessionDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let title = request.title.unwrap_or_default();
    let session = state
        .store
        .create_session("web", &user_id, &user_id, &title)
        .await
        .map_err(|_| ApiFailure::internal())?;
    let summary = state
        .store
        .session_summary(session)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::internal)?;
    Ok((StatusCode::CREATED, Json(session_dto(summary))))
}

async fn update_session(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Json(request): Json<UpdateSessionRequest>,
) -> ApiResult<Json<OkResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let changed = state
        .store
        .update_session(
            SessionId::new(id),
            &user_id,
            request.title.as_deref(),
            request.archived,
        )
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !changed {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(OkResponse { ok: true }))
}

async fn delete_session(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> ApiResult<Json<DeleteResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let deleted = state
        .store
        .delete_session(SessionId::new(id), &user_id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(DeleteResponse { deleted }))
}

async fn list_messages(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<MessageListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let info = state
        .store
        .session_info(SessionId::new(id))
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !info.is_some_and(|item| item.user_id == user_id && item.channel == "web") {
        return Err(ApiFailure::not_found());
    }
    let records = state
        .store
        .list_message_records(
            SessionId::new(id),
            query.before,
            query.limit.unwrap_or(100).min(500),
        )
        .await
        .map_err(|_| ApiFailure::internal())?;
    let messages = records
        .into_iter()
        .map(message_dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(MessageListResponse { messages }))
}

async fn get_message(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<i64>,
) -> ApiResult<Json<MessageDto>> {
    let user_id = require_user(&state, &jar).await?;
    let record = state
        .store
        .message_by_id(id)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::not_found)?;
    let info = state
        .store
        .session_info(record.session_id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !info.is_some_and(|item| item.user_id == user_id && item.channel == "web") {
        return Err(ApiFailure::not_found());
    }
    message_dto(record).map(Json)
}

async fn list_memories(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<MemoryQuery>,
) -> ApiResult<Json<MemoryListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let memories = state
        .store
        .list_memories(query.q.as_deref(), query.limit.unwrap_or(100).min(500))
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(Json(MemoryListResponse {
        memories: memories.into_iter().map(memory_dto).collect(),
    }))
}

async fn delete_memory(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
) -> ApiResult<Json<DeleteResponse>> {
    let _ = require_user(&state, &jar).await?;
    let deleted = state
        .store
        .delete_memory(id)
        .await
        .map_err(|error| match error {
            beanagent_memory::StoreError::NotFound(_) => ApiFailure::not_found(),
            _ => ApiFailure::internal(),
        })?;
    Ok(Json(DeleteResponse { deleted }))
}

async fn get_memory_file(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
) -> ApiResult<Json<MemoryFileResponse>> {
    let _ = require_user(&state, &jar).await?;
    let file = match name.as_str() {
        "MEMORY" | "MEMORY.md" => "MEMORY.md",
        "USER" | "USER.md" => "USER.md",
        _ => return Err(ApiFailure::not_found()),
    };
    let workspace = state.workspace.as_ref().ok_or_else(ApiFailure::internal)?;
    let content = tokio::task::spawn_blocking({
        let workspace = workspace.clone();
        let file = file.to_string();
        move || workspace.read_text(&file)
    })
    .await
    .map_err(|_| ApiFailure::internal())?
    .map_err(|error| match error {
        beanagent_tools::ToolError::NotFound(_) => ApiFailure::not_found(),
        _ => ApiFailure::internal(),
    })?;
    Ok(Json(MemoryFileResponse { name, content }))
}

async fn put_memory_file(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
    Json(request): Json<MemoryFileRequest>,
) -> ApiResult<Json<MemoryFileResponse>> {
    let _ = require_user(&state, &jar).await?;
    let file = match name.as_str() {
        "MEMORY" | "MEMORY.md" => "MEMORY.md",
        "USER" | "USER.md" => "USER.md",
        _ => return Err(ApiFailure::not_found()),
    };
    let workspace = state.workspace.as_ref().ok_or_else(ApiFailure::internal)?;
    tokio::task::spawn_blocking({
        let workspace = workspace.clone();
        let file = file.to_string();
        let content = request.content.clone();
        move || workspace.write_text(&file, &content).map_err(|_| ())
    })
    .await
    .map_err(|_| ApiFailure::internal())?
    .map_err(|_| ApiFailure::internal())?;
    Ok(Json(MemoryFileResponse {
        name,
        content: request.content,
    }))
}

async fn list_skills(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SkillListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let skills = state
        .skills
        .list()
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.name,
            description: skill.description,
        })
        .collect();
    Ok(Json(SkillListResponse { skills }))
}

async fn get_skill(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
) -> ApiResult<Json<SkillDetail>> {
    let _ = require_user(&state, &jar).await?;
    let skill = state
        .skills
        .get(&name)
        .map_err(|_| ApiFailure::not_found())?;
    Ok(Json(SkillDetail {
        name: skill.name,
        description: skill.description,
        content: skill.content,
        directory: skill.directory.to_string_lossy().into_owned(),
    }))
}

async fn list_skill_drafts(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SkillDraftListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let catalog = state.skills.clone();
    let drafts = tokio::task::spawn_blocking(move || catalog.list_drafts())
        .await
        .map_err(|_| ApiFailure::internal())?
        .map_err(skill_draft_api_error)?;
    Ok(Json(SkillDraftListResponse {
        drafts: drafts.iter().map(skill_draft_dto).collect(),
    }))
}

async fn approve_skill_draft(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(_request): Json<SkillDraftDecisionRequest>,
) -> ApiResult<Json<SkillDraftDecisionResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let decision = state
        .router
        .approve_draft(&id, &user_id)
        .await
        .map_err(router_skill_error)?;
    Ok(Json(SkillDraftDecisionResponse {
        id: decision.id,
        status: decision.status.as_str().into(),
    }))
}

async fn reject_skill_draft(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(_request): Json<SkillDraftDecisionRequest>,
) -> ApiResult<Json<SkillDraftDecisionResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let decision = state
        .router
        .reject_draft(&id, &user_id)
        .await
        .map_err(router_skill_error)?;
    Ok(Json(SkillDraftDecisionResponse {
        id: decision.id,
        status: decision.status.as_str().into(),
    }))
}

fn skill_draft_dto(draft: &SkillDraft) -> SkillDraftDto {
    SkillDraftDto {
        id: draft.id.clone(),
        name: draft.name.clone(),
        kind: draft.kind.as_str().into(),
        description: draft.description.clone(),
        content: draft.content.clone(),
        reason: draft.reason.clone(),
        status: draft.status.as_str().into(),
        created_at: draft.created_at.clone(),
    }
}

fn router_skill_error(error: beanagent_core::RouterError) -> ApiFailure {
    match error {
        beanagent_core::RouterError::Skill(error) => skill_draft_api_error(error),
        beanagent_core::RouterError::Forbidden(_) => ApiFailure::forbidden(),
        _ => ApiFailure::internal(),
    }
}

fn skill_draft_api_error(error: SkillError) -> ApiFailure {
    match error {
        SkillError::DraftNotFound(_) | SkillError::NotFound(_) => ApiFailure::not_found(),
        SkillError::DraftAlreadyPending(_)
        | SkillError::DraftConflict(_)
        | SkillError::AlreadyExists(_) => ApiFailure::new(
            StatusCode::CONFLICT,
            "skill_draft_conflict",
            "skill hoặc bản sửa đã thay đổi",
        ),
        SkillError::InvalidName(_)
        | SkillError::InvalidDescription(_)
        | SkillError::InvalidFrontmatter(_)
        | SkillError::MissingField(_)
        | SkillError::MissingBody
        | SkillError::InvalidDraft(_) => ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_skill_draft",
            "skill nháp không hợp lệ",
        ),
        SkillError::Io(_) | SkillError::DraftRandom(_) | SkillError::CatalogPoisoned => {
            ApiFailure::internal()
        }
    }
}

async fn task_owned_by_user(
    state: &WebState,
    task: &ScheduledTask,
    user_id: &str,
) -> ApiResult<bool> {
    let Some(session) = task.session_id else {
        return Ok(false);
    };
    let info = state
        .store
        .session_info(session)
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok(info.is_some_and(|info| info.user_id == user_id))
}

async fn list_tasks(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<TaskListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let tasks = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?;
    let mut result = Vec::new();
    for task in tasks {
        if task_owned_by_user(&state, &task, &user_id).await? {
            result.push(task_dto(task));
        }
    }
    Ok(Json(TaskListResponse { tasks: result }))
}

async fn create_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<TaskRequest>,
) -> ApiResult<(StatusCode, Json<TaskDto>)> {
    let user_id = require_user(&state, &jar).await?;
    let cron = request.cron.trim().to_string();
    let prompt = request.prompt.trim().to_string();
    let channel = request.channel.trim().to_string();
    let chat_id = request.chat_id.trim().to_string();
    if cron.is_empty() || prompt.is_empty() || channel.is_empty() || chat_id.is_empty() {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_task",
            "cron, prompt, channel và chat_id không được để trống",
        ));
    }
    if cron.len() > 256 || prompt.len() > 16_000 || channel.len() > 64 || chat_id.len() > 256 {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_task",
            "một trường của tác vụ quá dài",
        ));
    }
    let session = match request.session_id {
        Some(id) => {
            let info = state
                .store
                .session_info(SessionId::new(id))
                .await
                .map_err(|_| ApiFailure::internal())?
                .ok_or_else(ApiFailure::not_found)?;
            if info.user_id != user_id
                || info.channel != channel
                || info.chat_id != chat_id
                || info.archived
            {
                return Err(ApiFailure::not_found());
            }
            SessionId::new(id)
        }
        None => {
            if channel != "web" {
                return Err(ApiFailure::new(
                    StatusCode::BAD_REQUEST,
                    "session_required",
                    "task ngoài web phải chỉ định session_id thuộc user",
                ));
            }
            let active = state
                .store
                .find_active_session(&channel, &chat_id)
                .await
                .map_err(|_| ApiFailure::internal())?;
            if let Some(active) = active {
                let info = state
                    .store
                    .session_info(active)
                    .await
                    .map_err(|_| ApiFailure::internal())?;
                if info.is_some_and(|info| info.user_id == user_id && !info.archived) {
                    active
                } else {
                    state
                        .store
                        .ensure_session_for_user(&channel, &chat_id, &user_id, "")
                        .await
                        .map_err(|_| ApiFailure::internal())?
                }
            } else {
                state
                    .store
                    .ensure_session_for_user(&channel, &chat_id, &user_id, "")
                    .await
                    .map_err(|_| ApiFailure::internal())?
            }
        }
    };
    let allowed_tools = request
        .allowed_tools
        .into_iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect();
    let next_run = next_run_after(&cron, &state.config.agent.timezone, chrono::Utc::now())
        .map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_cron",
                "biểu thức cron không hợp lệ",
            )
        })?
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let task = state
        .store
        .create_task(NewScheduledTask {
            cron,
            prompt,
            session_id: Some(session),
            channel,
            chat_id,
            allowed_tools,
            next_run,
            enabled: request.enabled.unwrap_or(true),
        })
        .await
        .map_err(|_| ApiFailure::internal())?;
    Ok((StatusCode::CREATED, Json(task_dto(task))))
}

async fn update_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
    Json(request): Json<TaskUpdateRequest>,
) -> ApiResult<Json<TaskDto>> {
    let user_id = require_user(&state, &jar).await?;
    let existing = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .find(|task| task.id == id)
        .ok_or_else(ApiFailure::not_found)?;
    if !task_owned_by_user(&state, &existing, &user_id).await? {
        return Err(ApiFailure::not_found());
    }
    let task = state
        .store
        .set_task_enabled(id, request.enabled)
        .await
        .map_err(|_| ApiFailure::internal())?
        .ok_or_else(ApiFailure::not_found)?;
    Ok(Json(task_dto(task)))
}

async fn delete_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<u64>,
) -> ApiResult<Json<DeleteResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let existing = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .find(|task| task.id == id)
        .ok_or_else(ApiFailure::not_found)?;
    if !task_owned_by_user(&state, &existing, &user_id).await? {
        return Err(ApiFailure::not_found());
    }
    let deleted = state
        .store
        .delete_task(id)
        .await
        .map_err(|_| ApiFailure::internal())?;
    if !deleted {
        return Err(ApiFailure::not_found());
    }
    Ok(Json(DeleteResponse { deleted }))
}

async fn list_audit(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<AuditListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let Some(audit) = state.audit.as_ref() else {
        return Ok(Json(AuditListResponse {
            entries: Vec::new(),
        }));
    };
    let entries = audit
        .read_recent(query.before, query.limit.unwrap_or(100).min(500))
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();
    Ok(Json(AuditListResponse { entries }))
}

/// Dựng toàn bộ axum router; route `/api` được bảo vệ, UI có fallback riêng.
pub fn build_router(state: WebState) -> AxumRouter {
    let api = AxumRouter::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(auth_me))
        .route("/status", get(status))
        .route("/agents", get(list_agents))
        .route("/sessions", get(list_sessions).post(create_session))
        .route(
            "/sessions/{id}",
            patch(update_session).delete(delete_session),
        )
        .route("/sessions/{id}/messages", get(list_messages))
        .route("/messages/{id}", get(get_message))
        .route(
            "/memory/files/{name}",
            get(get_memory_file).put(put_memory_file),
        )
        .route("/memories", get(list_memories))
        .route("/memories/{id}", delete(delete_memory))
        .route("/skills", get(list_skills))
        .route("/skills/drafts", get(list_skill_drafts))
        .route("/skills/drafts/{id}/approve", post(approve_skill_draft))
        .route("/skills/drafts/{id}/reject", post(reject_skill_draft))
        .route("/skills/{name}", get(get_skill))
        .route("/tasks", get(list_tasks).post(create_task))
        .route("/tasks/{id}", patch(update_task).delete(delete_task))
        .route("/audit", get(list_audit))
        .route("/ws", any(ws_handler))
        .fallback(api_not_found)
        .method_not_allowed_fallback(api_not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES));

    AxumRouter::new()
        .nest("/api", api)
        .fallback(ui_handler)
        .layer(middleware::from_fn_with_state(state.clone(), request_guard))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn api_not_found() -> Response {
    ApiFailure::not_found().into_response()
}

async fn ui_handler(uri: Uri) -> Response {
    if uri.path() == "/api" || uri.path().starts_with("/api/") {
        return ApiFailure::not_found().into_response();
    }
    #[cfg(feature = "ui")]
    {
        embedded_response(uri.path())
    }
    #[cfg(not(feature = "ui"))]
    {
        let body = r#"<!doctype html><html lang="vi"><meta charset="utf-8"><title>BeanAgent</title><body><h1>BeanAgent</h1><p>UI sẽ được phục vụ khi build web.</p></body></html>"#;
        (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response()
    }
}

#[cfg(feature = "ui")]
#[derive(rust_embed::Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist/"]
#[allow_missing = true]
struct Assets;

#[cfg(feature = "ui")]
fn embedded_response(path: &str) -> Response {
    let relative = path.trim_start_matches('/');
    let (served_path, file) = match Assets::get(relative) {
        Some(file) => (relative, Some(file)),
        None => ("index.html", Assets::get("index.html")),
    };
    let Some(file) = file else {
        let body = r#"<!doctype html><html lang="vi"><meta charset="utf-8"><title>BeanAgent</title><body><h1>BeanAgent</h1><p>UI chưa được build; API vẫn hoạt động.</p></body></html>"#;
        return (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response();
    };
    let mime = mime_guess::from_path(served_path)
        .first_raw()
        .unwrap_or("application/octet-stream");
    let cache = if served_path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
        file.data,
    )
        .into_response()
}

/// WebSocket handler: xác thực trước khi `on_upgrade`, rồi mỗi socket có receiver riêng.
async fn ws_handler(
    State(state): State<WebState>,
    jar: CookieJar,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(token) = cookie_token(&jar) else {
        return ApiFailure::unauthorized().into_response();
    };
    let user_id = match state.auth.authenticate_token(&token).await {
        // So khớp với `[web].user_id` đang cấu hình: session phải thuộc đúng user mà
        // instance này phục vụ, nếu không một phiên cũ của user khác có thể lọt vào.
        Ok(info) if info.user_id == state.config.web.user_id => info.user_id,
        _ => return ApiFailure::unauthorized().into_response(),
    };
    let permit = match Arc::clone(&state.ws_connections).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return ApiFailure::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "too_many_connections",
                "đã đạt giới hạn kết nối WebSocket",
            )
            .into_response();
        }
    };
    upgrade
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| run_socket(socket, state, user_id, permit))
}

async fn run_socket(
    mut socket: WebSocket,
    state: WebState,
    user_id: String,
    _connection_permit: OwnedSemaphorePermit,
) {
    let mut router_events = state.router.events();
    let mut notifications = state.notifications.subscribe();
    let mut heartbeat = tokio::time::interval(WS_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_activity = Instant::now();
    if send_msg(
        &mut socket,
        &sync_msg(&state.router, &state.config, &user_id),
    )
    .await
    .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            event = router_events.recv() => match event {
                Ok(event) => {
                    if let Some(message) = map_run_event(event) {
                        // Lọc theo vai trò của người xem trước khi gửi: `prompt` của
                        // confirm có thể chứa nguyên văn lệnh shell, nên đây là dữ
                        // liệu nhạy cảm, không phải thứ để gửi rồi cho client ẩn.
                        if let Some(role) = event_role(&message)
                            && !event_visible_to(&state.config, &user_id, role)
                        {
                            last_activity = Instant::now();
                            continue;
                        }
                        if send_msg(&mut socket, &message).await.is_err() { break; }
                    }
                    last_activity = Instant::now();
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "WebSocket bị lag; gửi Sync mới");
                    if send_msg(&mut socket, &sync_msg(&state.router, &state.config, &user_id)).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            event = notifications.recv() => {
                match event {
                    Ok(message) => {
                        if send_msg(&mut socket, &message).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "WebSocket notification bị lag; gửi Sync mới");
                        if send_msg(&mut socket, &sync_msg(&state.router, &state.config, &user_id)).await.is_err() { break; }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            },
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                last_activity = Instant::now();
                match message {
                    WsMessage::Text(text) => {
                        if text.as_str().len() > MAX_WS_MESSAGE_BYTES {
                            let _ = send_msg(
                                &mut socket,
                                &ServerMsg::Error {
                                    session_id: None,
                                    run_id: None,
                                    code: "message_too_large".into(),
                                    message: "message vượt giới hạn".into(),
                                },
                            )
                            .await;
                            break;
                        }
                        match serde_json::from_str::<ClientMsg>(text.as_str()) {
                            Ok(ClientMsg::Ping) => {
                                if send_msg(&mut socket, &ServerMsg::Pong).await.is_err() { break; }
                            }
                            Ok(ClientMsg::Start { session_id, text }) => {
                                if text.len() > MAX_WS_MESSAGE_BYTES {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: Some(session_id),
                                        run_id: None,
                                        code: "text_too_large".into(),
                                        message: "nội dung vượt giới hạn".into(),
                                    }).await;
                                } else if handle_start(&state, &user_id, session_id, text).await.is_err() {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: Some(session_id),
                                        run_id: None,
                                        code: "invalid_session".into(),
                                        message: "session không hợp lệ".into(),
                                    }).await;
                                }
                            }
                            Ok(ClientMsg::Cancel { session_id }) => {
                                if handle_cancel(&state, &user_id, session_id).await.is_err() {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: Some(session_id),
                                        run_id: None,
                                        code: "invalid_session".into(),
                                        message: "session không hợp lệ".into(),
                                    }).await;
                                }
                            }
                            Ok(ClientMsg::Confirm { confirm_id, decision }) => {
                                let decision = match decision {
                                    DecisionDto::Allow => Decision::Allow,
                                    DecisionDto::AllowInSession => Decision::AllowInSession,
                                    DecisionDto::Deny => Decision::Deny,
                                };
                                if state.router.resolve_confirm(&confirm_id, decision, &user_id).await.is_err() {
                                    let _ = send_msg(&mut socket, &ServerMsg::Error {
                                        session_id: None,
                                        run_id: None,
                                        code: "confirm_invalid".into(),
                                        message: "confirm không hợp lệ".into(),
                                    }).await;
                                }
                            }
                            Err(_) => {
                                let _ = send_msg(&mut socket, &ServerMsg::Error {
                                    session_id: None,
                                    run_id: None,
                                    code: "invalid_message".into(),
                                    message: "message không hợp lệ".into(),
                                }).await;
                            }
                        }
                    }
                    WsMessage::Ping(payload) => {
                        if socket.send(WsMessage::Pong(payload)).await.is_err() { break; }
                    }
                    WsMessage::Close(_) => break,
                    WsMessage::Binary(_) => {
                        let _ = send_msg(&mut socket, &ServerMsg::Error {
                            session_id: None,
                            run_id: None,
                            code: "binary_not_allowed".into(),
                            message: "WebSocket chỉ nhận JSON text".into(),
                        }).await;
                    }
                    WsMessage::Pong(_) => {}
                }
            }
            _ = heartbeat.tick() => {
                if socket.send(WsMessage::Ping(Default::default())).await.is_err() { break; }
                if last_activity.elapsed() > WS_IDLE_TIMEOUT { break; }
            }
        }
    }
}

async fn handle_start(
    state: &WebState,
    user_id: &str,
    session_id: i64,
    text: String,
) -> Result<(), ()> {
    let info = state
        .store
        .session_info(SessionId::new(session_id))
        .await
        .map_err(|_| ())?
        .ok_or(())?;
    if info.user_id != user_id || info.channel != "web" || info.archived {
        return Err(());
    }
    state
        .router
        .submit(
            Incoming::new("web", user_id, user_id, text).with_session(SessionId::new(session_id)),
        )
        .await
        .map_err(|_| ())?;
    Ok(())
}

async fn handle_cancel(state: &WebState, user_id: &str, session_id: i64) -> Result<(), ()> {
    let info = state
        .store
        .session_info(SessionId::new(session_id))
        .await
        .map_err(|_| ())?
        .ok_or(())?;
    if info.user_id != user_id || info.channel != "web" {
        return Err(());
    }
    state
        .router
        .cancel_session(SessionId::new(session_id))
        .await;
    Ok(())
}

/// Dựng `Sync` cho **một** người xem, đã lọc theo RBAC của họ.
///
/// # Vì sao phải lọc
///
/// `Sync` vốn broadcast cho mọi kết nối. Khi web chỉ có một user `web:admin` thì
/// vô hại, nhưng nếu không lọc thì đây là **đường rò dữ liệu chéo vai trò**:
/// `finance-readonly` sẽ nhận `run_id` và cả `prompt` (nguyên văn lệnh shell) của
/// run thuộc Developer/Security-scan — đúng loại rò mà `docs/security-review.md`
/// đã cảnh báo cho `/api/audit`.
///
/// Lọc tại đây (trước khi tuần tự hoá) giữ đúng nguyên tắc: dữ liệu không thuộc
/// quyền thì **không có trong payload**, không phải "UI không hiển thị".
///
/// `None` nghĩa là RBAC tắt — không có khái niệm agent con nào để phân quyền, nên
/// ai cũng thấy (giữ nguyên hành vi một-người-dùng của M21).
fn sync_msg(router: &Router, config: &Config, user_id: &str) -> ServerMsg {
    let permissions = config.permissions_for(user_id);
    let visible = |role: &Option<String>| match role {
        None => true,
        Some(name) => config
            .role(name)
            .is_none_or(|role| permissions.can_view_agent(&role.tag_set())),
    };
    let snapshot = router.snapshot();
    ServerMsg::Sync {
        running: snapshot
            .running
            .into_iter()
            .filter(|item| visible(&item.role))
            .map(|item| RunningInfo {
                session_id: item.session_id.get(),
                run_id: item.run_id.as_str().to_string(),
                role: item.role,
            })
            .collect(),
        pending_confirms: snapshot
            .pending_confirms
            .into_iter()
            .filter(|item| visible(&item.role))
            .map(|item| PendingConfirm {
                confirm_id: item.confirm_id.as_str().to_string(),
                session_id: item.session_id.get(),
                run_id: item.run_id.as_str().to_string(),
                prompt: item.prompt,
                risk: match item.risk {
                    Risk::Safe => RiskDto::Safe,
                    Risk::Confirm => RiskDto::Confirm,
                    Risk::Dangerous => RiskDto::Dangerous,
                },
                allow_session_option: item.allow_session_option,
                timeout_seconds: item.timeout_seconds,
                role: item.role,
            })
            .collect(),
    }
}

/// Sự kiệp run này có thuộc vai trò mà `user_id` được phép thấy không?
///
/// `map_run_event` không có ngữ cảnh người xem nên không thể lọc; hàm này chạy ở
/// tầng socket, nơi đã xác thực `user_id`. Dùng cho sự kiện **trực tiếp** (không
/// qua `Sync`) để đường rò dữ liệu chéo vai trò không mở lại qua kênh broadcast.
fn event_visible_to(config: &Config, user_id: &str, role: &Option<String>) -> bool {
    let Some(name) = role else {
        return true;
    };
    let permissions = config.permissions_for(user_id);
    config
        .role(name)
        .is_none_or(|role| permissions.can_view_agent(&role.tag_set()))
}

/// Sự kiện `ConfirmRequest` mang `role`; các sự kiện khác không (chúng không chứa
/// dữ liệu thuộc domain của agent con) nên đi qua không cần lọc.
fn event_role(message: &ServerMsg) -> Option<&Option<String>> {
    match message {
        ServerMsg::ConfirmRequest { role, .. } => Some(role),
        _ => None,
    }
}

fn map_run_event(event: RunEvent) -> Option<ServerMsg> {
    Some(match event {
        RunEvent::Queued {
            session_id,
            run_id,
            position,
        } => ServerMsg::Queued {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            position,
        },
        RunEvent::Text {
            session_id,
            run_id,
            text,
        } => ServerMsg::Text {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            text,
        },
        RunEvent::TextDelta {
            session_id,
            run_id,
            text,
            index,
            reset,
        } => ServerMsg::TextDelta {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            text,
            index,
            reset,
        },
        RunEvent::ToolStart {
            session_id,
            run_id,
            id,
            tool,
            summary,
            args_preview,
            ..
        } => ServerMsg::ToolStart {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            id,
            tool,
            summary,
            args_preview,
        },
        RunEvent::ToolEnd {
            session_id,
            run_id,
            id,
            ok,
            output_preview,
            ..
        } => ServerMsg::ToolEnd {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            id,
            ok,
            output_preview,
        },
        RunEvent::ConfirmRequest {
            session_id,
            run_id,
            confirm_id,
            prompt,
            risk,
            allow_session_option,
            timeout_seconds,
            role,
        } => ServerMsg::ConfirmRequest {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            confirm_id: confirm_id.as_str().into(),
            prompt,
            risk: match risk {
                Risk::Safe => RiskDto::Safe,
                Risk::Confirm => RiskDto::Confirm,
                Risk::Dangerous => RiskDto::Dangerous,
            },
            allow_session_option,
            timeout_seconds,
            role,
        },
        RunEvent::ConfirmResolved {
            confirm_id,
            outcome,
            ..
        } => ServerMsg::ConfirmResolved {
            confirm_id: confirm_id.as_str().into(),
            outcome: match outcome {
                beanagent_types::ConfirmOutcome::Allowed => "allowed".into(),
                beanagent_types::ConfirmOutcome::Denied => "denied".into(),
                beanagent_types::ConfirmOutcome::Expired => "expired".into(),
            },
        },
        RunEvent::Final {
            session_id,
            run_id,
            message_id,
            ..
        } => ServerMsg::Final {
            session_id: session_id.get(),
            run_id: run_id.as_str().into(),
            message_id: message_id.unwrap_or_default(),
        },
        RunEvent::Error {
            session_id,
            run_id,
            code,
            message,
        } => ServerMsg::Error {
            session_id: Some(session_id.get()),
            run_id: Some(run_id.as_str().into()),
            code,
            message,
        },
    })
}

async fn send_msg(socket: &mut WebSocket, message: &ServerMsg) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    socket.send(WsMessage::text(text)).await.map_err(|_| ())
}
