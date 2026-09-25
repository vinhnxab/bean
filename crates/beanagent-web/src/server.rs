//! Axum application, REST, WebSocket và WebChannel cho M9.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
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
use beanagent_core::{Channel, Decision, Incoming, Router};
use beanagent_memory::{
    MemoryRecord, MessageRecord, NewScheduledTask, ScheduledTask, SessionSummary, Store,
};
use beanagent_security::AuditLog;
use beanagent_skills::SkillCatalog;
use beanagent_tools::WorkspaceFs;
use beanagent_types::{Config, Outbound, Risk, RunEvent, SessionId};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::api_types::*;
use crate::auth::{AuthError, AuthService, WEB_USER};
use croner::Cron;

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
        cron: task.cron,
        prompt: task.prompt,
        channel: task.channel,
        chat_id: task.chat_id,
        allowed_tools: task.allowed_tools,
        next_run: task.next_run,
        enabled: task.enabled,
    }
}

/// Parse cron và tính lần chạy kế tiếp UTC. M11 chỉ cung cấp metadata; M13 mới tick.
fn next_task_run(cron: &str) -> ApiResult<String> {
    let pattern = Cron::from_str(cron).map_err(|_| {
        ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_cron",
            "biểu thức cron không hợp lệ",
        )
    })?;
    let now = chrono::Utc::now();
    let next: chrono::DateTime<chrono::Utc> =
        pattern.find_next_occurrence(&now, false).map_err(|_| {
            ApiFailure::new(
                StatusCode::BAD_REQUEST,
                "invalid_cron",
                "không tìm thấy lần chạy kế tiếp",
            )
        })?;
    Ok(next.to_rfc3339())
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
        user_id: WEB_USER.to_string(),
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
    Ok(Json(StatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        model: state.config.llm.model.clone(),
        daily_token_budget: state.config.security.daily_token_budget,
        tokens_used: u64::from(usage.total()),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        channels: vec!["web".into()],
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

async fn list_tasks(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<TaskListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let tasks = state
        .store
        .list_tasks()
        .await
        .map_err(|_| ApiFailure::internal())?
        .into_iter()
        .map(task_dto)
        .collect();
    Ok(Json(TaskListResponse { tasks }))
}

async fn create_task(
    State(state): State<WebState>,
    jar: CookieJar,
    Json(request): Json<TaskRequest>,
) -> ApiResult<(StatusCode, Json<TaskDto>)> {
    let _ = require_user(&state, &jar).await?;
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
    let allowed_tools = request
        .allowed_tools
        .into_iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect();
    let next_run = next_task_run(&cron)?;
    let task = state
        .store
        .create_task(NewScheduledTask {
            cron,
            prompt,
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
    let _ = require_user(&state, &jar).await?;
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
    let _ = require_user(&state, &jar).await?;
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

async fn not_implemented(State(state): State<WebState>, jar: CookieJar) -> ApiResult<Response> {
    let _ = require_user(&state, &jar).await?;
    Ok((
        StatusCode::NOT_IMPLEMENTED,
        Json(NotImplementedResponse {
            code: "not_implemented".into(),
            message: "tính năng thuộc milestone sau".into(),
        }),
    )
        .into_response())
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
        .route("/skills/{name}", get(get_skill))
        .route("/skills/drafts", get(not_implemented))
        .route("/skills/drafts/{id}/approve", post(not_implemented))
        .route("/skills/drafts/{id}/reject", post(not_implemented))
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
        Ok(info) if info.user_id == WEB_USER => info.user_id,
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
    if send_msg(&mut socket, &sync_msg(&state.router))
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            event = router_events.recv() => match event {
                Ok(event) => {
                    if let Some(message) = map_run_event(event)
                        && send_msg(&mut socket, &message).await.is_err() { break; }
                    last_activity = Instant::now();
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "WebSocket bị lag; gửi Sync mới");
                    if send_msg(&mut socket, &sync_msg(&state.router)).await.is_err() { break; }
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
                        if send_msg(&mut socket, &sync_msg(&state.router)).await.is_err() { break; }
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

fn sync_msg(router: &Router) -> ServerMsg {
    let snapshot = router.snapshot();
    ServerMsg::Sync {
        running: snapshot
            .running
            .into_iter()
            .map(|item| RunningInfo {
                session_id: item.session_id.get(),
                run_id: item.run_id.as_str().to_string(),
            })
            .collect(),
        pending_confirms: snapshot
            .pending_confirms
            .into_iter()
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
            })
            .collect(),
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
