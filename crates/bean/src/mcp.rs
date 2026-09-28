//! `bean mcp` — chạy Bean ở chế độ **MCP server read-only** (M25).
//!
//! Agent khác (Cline, Cursor, OpenCode, Claude Code…) nối vào đây để đọc dữ liệu mà
//! Bean đã quản lý: log hạ tầng (`infra-read`), chi phí cloud (`billing-read`) và ghi
//! chú dài hạn của bạn (`memory-read`). **Không** có đường nào để ghi hay thực thi.
//!
//! # Hai transport
//!
//! * `bean mcp serve` — **stdio**: Cline/Cursor tự spawn tiến trình này. Đây là
//!   đường dùng hằng ngày; không cần mở cổng, không cần TLS.
//! * `bean mcp serve --http` — **streamable-HTTP/SSE** cho máy chạy từ xa, đặt sau
//!   reverse proxy TLS/Tailscale.
//!
//! # Cấu hình cần có trước
//!
//! ```toml
//! [mcp_server]
//! enabled = true
//!
//! [[mcp_clients]]
//! name = "cline"            # tên client
//! role = "finance-readonly" # role trong [[roles]]
//!
//! [agent]
//! # Bắt buộc: identity `mcp-client:<name>` phải map tới role ở trên,
//! # nếu không client sẽ là `no-access` và không thấy tool nào.
//! user_roles = { "mcp-client:cline" = "finance-readonly" }
//! ```
//!
//! Rồi chạy `bean auth mcp-token add cline` để sinh token (in một lần duy nhất).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};
use bean_core::mcp_server::{
    McpAuth, McpHttpService, McpRateLimiter, McpServerError, ServeContext, bearer_token,
    http_service, serve_stdio,
};
use bean_core::{Router, RouterDeps, SqliteStore, Store};
use bean_llm::{FakeProvider, LlmProvider};
use bean_skills::SkillCatalog;
use bean_types::Config;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::chat;
use crate::cli::McpCommand;

/// Chạy `bean mcp serve`.
pub async fn run(command: McpCommand, config_path: Option<&Path>) -> anyhow::Result<()> {
    let http = match command {
        McpCommand::Serve { http } => http,
    };
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    if !config.mcp_server.enabled {
        bail!("[mcp_server].enabled = false — hãy bật trong bean.toml trước khi chạy MCP server");
    }
    if config.mcp_clients.is_empty() {
        bail!(
            "chưa có [[mcp_clients]] nào — thêm ít nhất một client rồi chạy \
             `bean auth mcp-token add <tên>` để sinh token"
        );
    }
    let context = build_context(&config).await?;
    if http {
        serve_http(context, config.mcp_server.bind).await
    } else {
        serve_stdio(context)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }
}

/// Dựng store + registry + Router giống `serve`, chỉ khác là không bật kênh nào.
///
/// Không dùng lại `serve::run` vì lệnh đó bắt buộc phải bật web hoặc Telegram, còn
/// `mcp serve` là một tiến trình chỉ phục vụ MCP (đúng tinh thần "mỗi client stdio một
/// tiến trình riêng").
async fn build_context(config: &Config) -> anyhow::Result<ServeContext> {
    std::fs::create_dir_all(&config.data.dir)
        .with_context(|| format!("không tạo được data.dir {}", config.data.dir.display()))?;
    // Một writer duy nhất cho SQLite (D8.2): `store` và `McpAuth` dùng **cùng** một
    // `SqliteStore`, không mở connection thứ hai.
    let store = Arc::new(
        SqliteStore::open(&chat::store_path(config))
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("không mở được SQLite store")?,
    );
    let skills_root = chat::expand_tilde(&config.data.dir).join("skills");
    let skills = SkillCatalog::load_with_paths(
        &[std::path::PathBuf::from("skills")],
        skills_root,
        std::path::PathBuf::from("skills/_drafts"),
    );
    let built = chat::build_registry(config, store.clone(), skills, None).await?;
    let audit = chat::build_audit(config);
    // K24: nhật ký riêng của MCP, tách khỏi `audit.jsonl` để phát hiện lạm dụng không
    // phải lọc giữa các kênh. Vẫn ghi **song song** vào `audit.jsonl` (D16.10).
    let mcp_audit = build_mcp_audit(config);
    // K24: limiter dựng từ `[mcp_server]`. `0` ⇒ tắt trần lưu lượng nhưng vẫn giữ
    // ngưỡng chống dò token (xem `McpRateLimiter`).
    let limiter = if config.mcp_server.rate_limit_per_minute == 0 {
        tracing::warn!(
            "[mcp_server].rate_limit_per_minute = 0 — chỉ chống dò token, KHÔNG giới hạn \
             lưu lượng; chỉ chấp nhận được khi endpoint không công khai"
        );
        McpRateLimiter::unlimited()
    } else {
        McpRateLimiter::new(&config.mcp_server)
    };
    // MCP không cần LLM: client gọi tool trực tiếp, không có vòng lặp agent. Dùng
    // `FakeProvider` để Router dựng được mà không cần API key.
    let llm: Arc<dyn LlmProvider> = Arc::new(FakeProvider::echo());
    let store_dyn: Arc<dyn Store> = store.clone();
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store_dyn,
        registry: Arc::new(built.registry),
        llm,
        audit: audit.clone(),
        skills_index: String::new(),
        skills: None,
    }));
    Ok(ServeContext {
        auth: Arc::new(McpAuth::new(store)),
        router,
        audit,
        mcp_audit,
        config: Arc::new(config.clone()),
        shutdown: CancellationToken::new(),
        limiter: Arc::new(limiter),
    })
}

/// Mở `audit/mcp.jsonl`; lỗi chỉ cảnh báo trên stderr, không chặn server.
///
/// Ghi ra **stderr** chứ không `println!` vì ở `mcp serve` (stdio) stdout chính là kênh
/// JSON-RPC (D16.9).
fn build_mcp_audit(config: &Config) -> Option<Arc<bean_security::AuditLog>> {
    let audit_dir = chat::expand_tilde(&config.data.dir).join("audit");
    match bean_security::AuditLog::open_named(&audit_dir, MCP_AUDIT_FILE) {
        Ok(log) => Some(Arc::new(log)),
        Err(error) => {
            eprintln!(
                "cảnh báo: không mở được nhật ký MCP ở {}: {error}",
                audit_dir.join(MCP_AUDIT_FILE).display()
            );
            None
        }
    }
}

/// Tên nhật ký riêng của MCP trong `data.dir/audit/` (K24).
pub const MCP_AUDIT_FILE: &str = "mcp.jsonl";

/// State của HTTP transport: context + **service đã dựng, theo từng client**.
///
/// # Vì sao phải cache service thay vì dựng mỗi request
///
/// `StreamableHttpService` giữ `LocalSessionManager` **bên trong nó**. Nếu dựng service
/// mới cho mỗi HTTP request thì mỗi request lại một `SessionManager` rỗng ⇒ session MCP
/// tạo ở `initialize` biến mất ngay, và client không bao giờ nhận được `Mcp-Session-Id`
/// để gọi tiếp `tools/call`. Đây chính là lý do Bean cần cache theo client: vừa giữ
/// session sống, vừa bảo đảm session của client A không bao giờ đọc được bởi client B.
#[derive(Clone)]
struct HttpState {
    context: ServeContext,
    services: Arc<tokio::sync::Mutex<std::collections::HashMap<String, McpHttpService>>>,
}

impl HttpState {
    /// Lấy (hoặc dựng lần đầu) service của một client đã xác thực.
    async fn service_for(
        &self,
        identity: bean_core::mcp_server::McpClientIdentity,
    ) -> Result<McpHttpService, bean_core::mcp_server::McpServerError> {
        let mut services = self.services.lock().await;
        // Khoá phải **giống hệt** lúc insert, nên cả hai đều đi qua `services_key` —
        // tra `identity.name` ở một chỗ và `identity.user_id` ở chỗ kia sẽ khiến cache
        // không bao giờ trúng và session lại mất mỗi request.
        let key = services_key(&identity);
        if let Some(service) = services.get(&key) {
            return Ok(service.clone());
        }
        let service = http_service(&self.context, identity)?;
        services.insert(key, service.clone());
        Ok(service)
    }
}

/// Khoá cache: identity đầy đủ (`mcp-client:<name>`) chứ không phải tên trần, để sau
/// này thêm chính sách theo vai trò mà không đụng khoá cũ.
fn services_key(identity: &bean_core::mcp_server::McpClientIdentity) -> String {
    identity.user_id.clone()
}

/// Chạy transport HTTP: mỗi request được xác thực **trước** khi chạm MCP service.
async fn serve_http(context: ServeContext, bind: SocketAddr) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("không bind được {bind}"))?;
    let local_addr = listener
        .local_addr()
        .context("đọc địa chỉ listener thất bại")?;
    tracing::info!(%local_addr, "Bean MCP server (HTTP) đang lắng nghe");
    let app = axum::Router::new()
        .route("/mcp", axum::routing::any(handle))
        .with_state(HttpState {
            context,
            services: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        });
    // `into_make_service_with_connect_info` là điều kiện tiên quyết cho giới hạn theo IP
    // (K24): không có nó thì `ConnectInfo` không bao giờ được chèn vào request và mọi
    // request sẽ bị gán cùng một khoá loopback — tức là **không giới hạn được gì**.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
    })
    .await
    .context("MCP HTTP server dừng lỗi")
}

/// Một request HTTP tới endpoint MCP: giới hạn tần suất + xác thực Bearer, rồi
/// chuyển tiếp cho `StreamableHttpService` của **đúng client đó**.
async fn handle(
    axum::extract::State(state): axum::extract::State<HttpState>,
    peer: Option<axum::extract::Extension<axum::extract::ConnectInfo<SocketAddr>>>,
    headers: axum::http::HeaderMap,
    request: axum::extract::Request,
) -> axum::response::Response {
    let context = &state.context;
    // IP đã chuẩn hoá ở tầng socket. Không đọc `X-Forwarded-For`: header đó do client
    // tự chọn, tin vào nó là để kẻ tấn công tự chọn khoá IP nào bị khoá (mục 15.7 giữ
    // mô hình "không tin tiêu đề do người dùng kiểm soát").
    let ip = peer.map_or(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        |axum::extract::Extension(axum::extract::ConnectInfo(peer))| peer.ip(),
    );
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| bearer_token(Some(raw)));
    // `authenticate_http` tự kiểm tra giới hạn **trước** rồi mới tra DB (K24).
    let identity = match context.authenticate_http(token, ip).await {
        Ok(identity) => identity,
        Err(McpServerError::RateLimited(retry_after)) => {
            tracing::warn!(%ip, retry_after = retry_after.as_secs(), "MCP bị giới hạn tần suất");
            // Ghi vào `mcp.jsonl` chứ không chỉ `tracing`: mục tiêu của nhật ký riêng là
            // để **phát hiện lạm dụng tần suất**, mà đây chính là dấu vết của hành vi đó
            // (sự kiện này chưa tới handler nên không được `audit_event` ghi).
            context.record_denied(ip, "mcp_rate_limited", retry_after.as_secs());
            return too_many_requests(retry_after);
        }
        Err(error) => {
            tracing::warn!(%ip, error = %error, "từ chối request MCP không xác thực được");
            context.record_denied(ip, "mcp_auth_failed", 0);
            return unauthorized("token không hợp lệ hoặc đã hết hạn");
        }
    };
    let service = match state.service_for(identity).await {
        Ok(service) => service,
        Err(error) => {
            tracing::error!(error = %error, "không dựng được MCP service");
            return status_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "lỗi nội bộ");
        }
    };
    // `StreamableHttpService` là `tower::Service`; `oneshot` chạy đúng một request rồi
    // trả về. Body của nó là `BoxBody<Bytes, Infallible>` nên chuyển sang
    // `axum::body::Body` là an toàn (không thể lỗi vì `Infallible`).
    //
    // PHẢI giữ lại status **và header** của response: `Mcp-Session-Id` mà rmcp gắn vào
    // phản hồi `initialize` chính là thứ client dùng để gọi tiếp `tools/call`; dựng
    // `Response::new(body)` từ đầu sẽ âm thầm làm mất nó và client không bao giờ gọi
    // được request thứ hai.
    use tower::ServiceExt;
    match service.oneshot(request).await {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            let mut out = axum::response::Response::new(axum::body::Body::new(body));
            *out.status_mut() = parts.status;
            *out.headers_mut() = parts.headers;
            out
        }
        // `Service::Error` của service là `Infallible` ⇒ nhánh này không bao giờ xảy ra.
        Err(never) => match never {},
    }
}

/// 429 chuẩn của streamable-HTTP: `Retry-After` để client tự thử lại đúng nhịp.
///
/// Trả `429` chứ không phải `401` là cố ý — `401` khiến Cline/Cursor báo "token sai"
/// và người dùng đi sửa cấu hình vô ích, trong khi nguyên nhân thật là chạy quá nhanh.
fn too_many_requests(retry_after: std::time::Duration) -> axum::response::Response {
    use axum::http::header::{CACHE_CONTROL, RETRY_AFTER};
    let seconds = retry_after.as_secs().max(1);
    let mut response = status_response(
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        &format!("quá nhiều request; thử lại sau {seconds} giây"),
    );
    let headers = response.headers_mut();
    for (name, value) in [(RETRY_AFTER, seconds), (CACHE_CONTROL, 0)] {
        // `value` là số nguyên ≤ 86400 nên luôn parse được; bỏ qua lỗi thay vì
        // dùng `unwrap` (mục 0.8) — thiếu header chỉ làm client thử lại sớm hơn.
        let _ = headers.insert(
            name,
            axum::http::HeaderValue::try_from(value.to_string())
                .unwrap_or_else(|_| axum::http::HeaderValue::from_static("1")),
        );
    }
    response
}

/// 401 chuẩn của streamable-HTTP: client hiểu đây là "token sai", không phải lỗi server.
fn unauthorized(message: &str) -> axum::response::Response {
    use axum::http::header::{CACHE_CONTROL, WWW_AUTHENTICATE};
    let mut response = status_response(axum::http::StatusCode::UNAUTHORIZED, message);
    let headers = response.headers_mut();
    if let Ok(value) = axum::http::HeaderValue::try_from("Bearer") {
        headers.insert(WWW_AUTHENTICATE, value);
    }
    if let Ok(value) = axum::http::HeaderValue::try_from("no-store") {
        headers.insert(CACHE_CONTROL, value);
    }
    response
}

/// Dựng response đơn giản mà không dùng `unwrap` (mục 0.8).
fn status_response(status: axum::http::StatusCode, message: &str) -> axum::response::Response {
    let mut response = axum::response::Response::new(axum::body::Body::from(message.to_string()));
    *response.status_mut() = status;
    response
}
