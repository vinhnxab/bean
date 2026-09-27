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
    /// Audit log (tuỳ chọn).
    pub audit: Option<Arc<beanagent_security::AuditLog>>,
    /// Cấu hình đầy đủ (chỉ đọc `mcp_server` + tra client).
    pub config: Arc<beanagent_types::Config>,
    /// Token huỷ: dừng êm server.
    pub shutdown: CancellationToken,
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
            },
        ))
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
    let identity = ctx.auth.authenticate(&ctx.config, &token).await?;
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
