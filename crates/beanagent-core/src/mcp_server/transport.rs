//! Transport cho MCP server của Bean (M25): stdio và streamable-HTTP.
//!
//! # Xác thực ở đâu — trước handshake
//!
//! Cả hai transport đều **xác thực token trước khi tạo handler**, tức là trước khi
//! client gửi `initialize`. Đó là hình thái mạnh hơn "từ chối ở bước handshake": client
//! không hề biết Bean có tồn tại, không thấy danh sách tool, và không gửi được tham số
//! nào xuống tầng dưới.
//!
//! * **stdio**: token lấy từ biến môi trường `BEANAGENT_MCP_TOKEN` mà client (Cline…)
//!   truyền khi spawn tiến trình. Thiếu/sai ⇒ tiến trình **thoát ngay**.
//! * **HTTP**: token lấy từ header `Authorization: Bearer …`; thiếu/sai ⇒ `401` **trước**
//!   khi request tới `StreamableHttpService`.
//!
//! # Vì sao không dùng cookie phiên web
//!
//! MCP client là chương trình CLI/IDE, không phải trình duyệt — nó không quản lý cookie.
//! Token Bearer là hình thức đúng cho client như vậy, và nó vẫn dùng **cùng** cơ chế
//! hash lưu trong `beanagent.db` (mục 15.7: không tạo hệ xác thực song song).

use std::sync::Arc;

use rmcp::{
    ServiceExt,
    transport::{
        io::stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, session::local::LocalSessionManager,
            tower::StreamableHttpService as HttpService,
        },
    },
};
use tokio_util::sync::CancellationToken;

use crate::Router;
use crate::mcp_server::auth::McpAuth;

/// Biến môi trường chứa token cho transport stdio.
pub const TOKEN_ENV: &str = "BEANAGENT_MCP_TOKEN";

/// Lỗi khởi động MCP server.
#[derive(Debug, thiserror::Error)]
pub enum McpServerError {
    /// Token thiếu hoặc không hợp lệ.
    #[error("xác thực MCP client thất bại: {0}")]
    Auth(#[from] crate::mcp_server::auth::McpAuthError),
    /// Lỗi transport/serve.
    #[error("MCP server: {0}")]
    Transport(String),
    /// Cấu hình sai (thiếu client, bind sai…).
    #[error("cấu hình MCP server không hợp lệ: {0}")]
    Config(String),
    /// Vượt giới hạn tần suất trên transport HTTP (K24).
    ///
    /// Tách riêng [`Self::Auth`] để tầng HTTP trả `429` + `Retry-After` thay vì `401`:
    /// client hiểu "chậm lại" khác hẳn "token của bạn sai", và `429` là mã chuẩn để
    /// client MCP tự thử lại thay vì báo lỗi cho người dùng.
    ///
    /// `Duration` không implement `Display` nên phải tự định dạng.
    #[error("vượt giới hạn tần suất MCP; thử lại sau {} giây", .0.as_secs())]
    RateLimited(std::time::Duration),
}

/// Bối cảnh dùng chung cho cả hai transport.
///
/// `Clone` rẻ (mọi trường là `Arc`/`CancellationToken`/`Config`): axum yêu cầu state
/// `Clone` để nhân bản router cho mỗi connection.
#[derive(Clone)]
pub struct ServeContext {
    /// Router giữ quyền quyết định.
    pub router: Arc<Router>,
    /// Bộ xác thực token.
    pub auth: Arc<McpAuth>,
    /// Audit log **chung** (audit.jsonl) — giữ nguyên để `/api/audit` và trang Audit
    /// trong UI không mất bản ghi MCP (K24, xem `docs/decisions.md` D16.10).
    pub audit: Option<Arc<beanagent_security::AuditLog>>,
    /// Nhật ký riêng của MCP (audit/mcp.jsonl) — mục tiêu K24: phát hiện lạm dụng
    /// không phải lọc giữa các kênh khác.
    pub mcp_audit: Option<Arc<beanagent_security::AuditLog>>,
    /// Cấu hình đầy đủ (chỉ đọc `mcp_server` + tra client).
    pub config: Arc<beanagent_types::Config>,
    /// Token huỷ: dừng êm server.
    pub shutdown: CancellationToken,
    /// Giới hạn tần suất — **chỉ** dùng ở `authenticate_http` (K24).
    pub limiter: Arc<crate::mcp_server::guard::McpRateLimiter>,
}

impl ServeContext {
    /// Dựng handler cho client đã xác thực.
    ///
    /// # Errors
    /// Trả lỗi nếu không resolve được quyền cho identity (state nội bộ hỏng).
    pub fn handler_for(
        &self,
        identity: crate::mcp_server::auth::McpClientIdentity,
    ) -> Result<crate::mcp_server::BeanMcpHandler, McpServerError> {
        let permissions = self
            .router
            .mcp_permissions(&identity.user_id)
            .map_err(|error| McpServerError::Config(error.to_string()))?;
        Ok(crate::mcp_server::BeanMcpHandler::new(
            crate::mcp_server::HandlerDeps {
                router: self.router.clone(),
                identity,
                permissions,
                audit: self.audit.clone(),
                mcp_audit: self.mcp_audit.clone(),
            },
        ))
    }

    /// Xác thực cho transport **stdio**.
    ///
    /// Cố ý **không** có tham số limiter (K24): token tới từ biến môi trường mà chính
    /// tiến trình của bạn đặt, nên không có bề mặt tấn công từ xa để giới hạn. Thêm
    /// limiter ở đây sẽ chỉ làm vỡ phiên làm việc dài.
    ///
    /// # Errors
    /// Giống [`McpAuth::authenticate`].
    pub async fn authenticate_stdio(
        &self,
        token: &str,
    ) -> Result<crate::mcp_server::auth::McpClientIdentity, McpServerError> {
        self.auth
            .authenticate(&self.config, token)
            .await
            .map_err(Into::into)
    }

    /// Ghi sự kiện **bị từ chối trước khi** tới handler (auth fail, rate-limit) vào cả
    /// hai nhật ký.
    ///
    /// Ở thời điểm này chưa có danh tính client (token chưa hợp lệ) nên `channel` ghi là
    /// `mcp-server` và IP nằm trong `args`. `retry_after_seconds = 0` nghĩa là không
    /// phải lỗi tần suất.
    ///
    /// Vì sao ở đây chứ không ở [`crate::mcp_server::BeanMcpHandler`]: handler chỉ được
    /// dựng **sau** bước xác thực, mà sự kiện cần ghi chính xảy ra **trước** bước đó.
    /// Ghi cả hai file để nhất quán với D16.10.
    pub fn record_denied(&self, ip: std::net::IpAddr, event: &str, retry_after_seconds: u64) {
        let mut entry = beanagent_security::entry_now(
            0,
            "mcp-server",
            event,
            &serde_json::json!({
                "via": "mcp",
                "ip": ip.to_string(),
                "retry_after_seconds": retry_after_seconds,
            }),
        );
        entry.ok = Some(false);
        entry.decision = "deny";
        entry.decided_by = "mcp-server".into();
        for (target, log) in [
            ("audit.jsonl", self.audit.as_ref()),
            ("mcp.jsonl", self.mcp_audit.as_ref()),
        ] {
            if let Some(log) = log
                && let Err(error) = log.record(&entry)
            {
                tracing::warn!(error = %error, file = target, "không ghi được audit MCP");
            }
        }
    }

    /// Xác thực cho transport **HTTP** — có giới hạn tần suất (K24).
    ///
    /// Thứ tự cố ý là **giới hạn trước, tra DB sau**: `authenticate` băm token rồi tra
    /// bảng `mcp_clients`; nếu tra trước thì một kẻ dò token có thể bắn hàng loạt truy
    /// vấn vào SQLite.
    ///
    /// # Errors
    /// [`McpServerError::RateLimited`] khi vượt ngưỡng; [`McpServerError::Auth`] khi
    /// token sai/hết hạn.
    pub async fn authenticate_http(
        &self,
        token: Option<&str>,
        ip: std::net::IpAddr,
    ) -> Result<crate::mcp_server::auth::McpClientIdentity, McpServerError> {
        use crate::mcp_server::guard::LimitVerdict;
        if let LimitVerdict::Limited { retry_after, .. } = self.limiter.check(token, ip) {
            return Err(McpServerError::RateLimited(retry_after));
        }
        let Some(token) = token else {
            // Không có token là một lần thất bại xác thực: phải tính vào bộ đếm để
            // kẻ dò không lách được bằng cách bỏ trống header.
            self.limiter.record_auth_failure(None, ip);
            return Err(McpServerError::Auth(
                crate::mcp_server::auth::McpAuthError::InvalidToken,
            ));
        };
        match self.auth.authenticate(&self.config, token).await {
            Ok(identity) => {
                self.limiter.record_auth_success(token);
                self.limiter.record_request(token, ip);
                Ok(identity)
            }
            Err(error) => {
                self.limiter.record_auth_failure(Some(token), ip);
                Err(McpServerError::Auth(error))
            }
        }
    }
}

/// Bóc token khỏi header `Authorization: Bearer …`.
#[must_use]
pub fn bearer_token(header: Option<&str>) -> Option<&str> {
    let raw = header?.trim();
    let (scheme, value) = raw.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| value.trim())
        .filter(|token| !token.is_empty())
}

/// Chạy MCP server qua **stdio** (Bean cùng máy với IDE).
///
/// # Errors
/// * [`McpServerError::Auth`] — `BEANAGENT_MCP_TOKEN` thiếu/sai ⇒ **thoát trước khi
///   đọc stdin**, tức client không thể nói chuyện với Bean.
/// * [`McpServerError::Config`] — không resolve được quyền cho client.
/// * [`McpServerError::Transport`] — lỗi khi phục vụ.
pub async fn serve_stdio(ctx: ServeContext) -> Result<(), McpServerError> {
    let token = std::env::var(TOKEN_ENV)
        .map_err(|_| McpServerError::Auth(crate::mcp_server::auth::McpAuthError::InvalidToken))?;
    // Đường stdio **không** đi qua limiter (K24) — xem `authenticate_stdio`.
    let identity = ctx.authenticate_stdio(&token).await?;
    tracing::info!(client = %identity.name, role = %identity.role, "MCP server stdio sẵn sàng");
    let handler = ctx.handler_for(identity)?;
    handler.audit_event("mcp_handshake", true, None);
    let (read, write) = stdio();
    let service = handler
        .serve_with_ct((read, write), ctx.shutdown.clone())
        .await
        .map_err(|error| McpServerError::Transport(error.to_string()))?;
    // `serve` chỉ trả về khi transport đóng; chờ tới lúc đó nên stdio giữ tiến trình sống.
    // `waiting()` trả `Result<QuitReason, JoinError>`: `Ok` = service kết thúc bình thường
    // (EOF ở stdin — trường hợp thường gặp khi Cline đóng phiên làm việc). `QuitReason`
    // không phải lỗi nên bỏ qua, chỉ `JoinError` mới thành lỗi transport.
    service
        .waiting()
        .await
        .map(|_quit| ())
        .map_err(|error| McpServerError::Transport(error.to_string()))
}

/// Router HTTP cho endpoint MCP (streamable-HTTP/SSE).
///
/// `S = BeanMcpHandler` (không bọc `Arc`): factory của `StreamableHttpService` trả `S`
/// theo **giá trị**, nên `Clone` rẻ là đủ — xem [`http_service`].
pub type McpHttpService = HttpService<crate::mcp_server::BeanMcpHandler, LocalSessionManager>;

/// Dựng service streamable-HTTP cho **một** client đã xác thực.
///
/// Mỗi client có service riêng ⇒ session MCP tách biệt theo client, và quyền lấy từ
/// danh tính của chính service đó (không dùng biến toàn cục ⇒ không trộn quyền khi
/// nhiều client dùng chung một tiến trình).
///
/// # Errors
/// [`McpServerError::Config`] khi không resolve được quyền cho client.
pub fn http_service(
    ctx: &ServeContext,
    identity: crate::mcp_server::auth::McpClientIdentity,
) -> Result<McpHttpService, McpServerError> {
    let config = StreamableHttpServerConfig::default()
        .with_cancellation_token(ctx.shutdown.child_token())
        // Chặn DNS rebinding: mặc định rmcp chỉ nhận `Host` loopback; chỉ mở thêm khi
        // người dùng khai báo tường minh trong `allowed_hosts`.
        .with_allowed_hosts(host_allowlist(&ctx.config.mcp_server));
    // Handler được dựng **một lần** rồi clone cho từng session MCP. `McpServerError`
    // không `Clone`, nên ta clone trước khi kết hợp với `?` — nhờ vậy mọi session
    // dùng chung đúng một đối tượng handler, trạng thái quyền không thể lệch.
    let handler = ctx.handler_for(identity)?;
    Ok(HttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    ))
}

/// Danh sách `Host` được phép nhận: `Host` của `public_origin` + `allowed_hosts`.
#[must_use]
pub fn host_allowlist(settings: &beanagent_types::config::McpServerConfigSettings) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    if let Some(host) = authority_of(&settings.public_origin) {
        hosts.push(host);
    }
    hosts.extend(
        settings
            .allowed_hosts
            .iter()
            .map(|host| host.trim().to_string())
            .filter(|host| !host.is_empty()),
    );
    if hosts.is_empty() {
        // Không khai gì ⇒ chỉ loopback (mặc định an toàn của rmcp khi danh sách rỗng).
        hosts.push("127.0.0.1".to_string());
        hosts.push("localhost".to_string());
    }
    hosts
}

/// Lấy `host[:port]` từ một URL (`http://127.0.0.1:7879` → `127.0.0.1:7879`).
fn authority_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.trim();
    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use beanagent_types::config::McpServerConfigSettings;

    #[test]
    fn bearer_token_requires_bearer_scheme() {
        assert_eq!(bearer_token(Some("Bearer abc123")), Some("abc123"));
        assert_eq!(bearer_token(Some("bearer  abc123 ")), Some("abc123"));
        assert_eq!(bearer_token(Some("Basic abc123")), None);
        assert_eq!(bearer_token(Some("abc123")), None);
        assert_eq!(bearer_token(Some("Bearer   ")), None);
        assert_eq!(bearer_token(None), None);
    }

    #[test]
    fn host_allowlist_defaults_to_loopback_only() {
        let settings = McpServerConfigSettings::default();
        let hosts = host_allowlist(&settings);
        assert!(hosts.contains(&"127.0.0.1:7879".to_string()), "{hosts:?}");
    }

    #[test]
    fn host_allowlist_can_be_widened_explicitly() {
        let settings = McpServerConfigSettings {
            public_origin: "https://bean.example.com".to_string(),
            allowed_hosts: vec!["proxy.internal".to_string()],
            ..McpServerConfigSettings::default()
        };
        let hosts = host_allowlist(&settings);
        assert!(hosts.contains(&"bean.example.com".to_string()));
        assert!(hosts.contains(&"proxy.internal".to_string()));
    }
}
