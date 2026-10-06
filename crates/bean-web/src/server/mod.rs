//! Axum application, REST, WebSocket và WebChannel cho M9.
//!
//! # Vì sao là thư mục chứ không phải một file
//!
//! File `server.rs` trước đây dài 1897 dòng và trộn **bốn** loại việc không liên quan:
//! middleware bảo mật, ~30 handler REST, vòng lặp WebSocket, và dựng router. Hậu quả
//! cụ thể: sửa kiểm tra `Origin` (mục 15.7 — lớp rủi ro nghiêm trọng nhất vì agent có
//! quyền chạy lệnh) buộc phải mở file chứa cả `run_socket` và 30 handler.
//!
//! # Cách chia — theo ranh giới bảo mật và nghiệp vụ
//!
//! ```text
//! server/
//! ├─ mod.rs                WebState, ApiFailure, WebChannel, bản đồ module
//! ├─ guard.rs              Origin/Host/CSRF + security headers  (mục 15.7)
//! ├─ dto.rs                chuyển record của store → DTO JSON
//! ├─ routes.rs             dựng axum router
//! ├─ ui_assets.rs          phục vụ UI nhúng + SPA fallback
//! ├─ ws.rs                 vòng lặp WebSocket (`/api/ws`)
//! └─ rest_*.rs             handler REST, mỗi file một nhóm tài nguyên
//!    ├─ rest_auth_status.rs      đăng nhập, trạng thái, danh sách agent
//!    ├─ rest_sessions.rs         phiên hội thoại + message
//!    ├─ rest_memories.rs         bộ nhớ dài hạn + MEMORY.md/USER.md
//!    ├─ rest_skills.rs           skills và duyệt skill nháp
//!    ├─ rest_tasks.rs            tác vụ định kỳ
//!    ├─ rest_audit_tools.rs      audit log + danh sách tool
//!    └─ rest_usage_system.rs     lịch sử token + thông tin hệ thống
//! ```
//!
//! # Bất biến phải giữ khi tách
//!
//! * `/api/*` không tồn tại phải trả **404 JSON**, không rơi vào SPA fallback (mục
//!   22.17) — ranh giới này nằm ở `routes.rs` + `ui_assets.rs`.
//! * WebSocket kiểm tra `Origin` + cookie **trước** khi nâng cấp (chống cross-site
//!   WebSocket hijacking, mục 22.15) — nằm ở `guard.rs` và `ws.rs`.
//! * Request thay đổi dữ liệu bắt buộc `Content-Type: application/json`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bean_core::{Channel, Router};
use bean_memory::Store;
use bean_security::AuditLog;
use bean_skills::SkillCatalog;
use bean_tools::WorkspaceFs;
use bean_types::{Config, Outbound};
use tokio::sync::{Semaphore, broadcast};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::api_types::{ApiError, ServerMsg};
use crate::auth::AuthService;

// ---------------------------------------------------------------------------
// Khai báo module — lý do chia nằm trong doc đầu file
// ---------------------------------------------------------------------------

mod dto;
mod guard;
mod rest_audit_tools;
mod rest_auth_status;
mod rest_memories;
mod rest_sessions;
mod rest_skills;
mod rest_tasks;
mod rest_usage_system;
mod routes;
mod ui_assets;
mod ws;

// ---------------------------------------------------------------------------
// Re-export — API công khai của crate
// ---------------------------------------------------------------------------

pub use guard::request_guard;
pub use guard::security_headers;
pub use routes::build_router;

/// Xác thực cookie cho REST handler — mọi handler dùng chung một cách.
pub(crate) use dto::{cookie_token, require_user};

/// Chuyển record của store sang DTO JSON; lỗi serialize trả 500 an toàn.
pub(crate) use dto::{memory_dto, message_dto, session_dto, task_dto};

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
