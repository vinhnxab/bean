//! MCP client stdio tích hợp Bean (agents.md mục 16).
//!
//! Mỗi server được spawn thành tiến trình con, handshake bằng `rmcp`, discovery qua
//! `tools/list`, rồi đăng ký từng tool thành `mcp__<server>__<tool>`. Server lỗi hoặc treo
//! chỉ bị log và bỏ qua; registry built-in vẫn dùng được để agent khởi động.

use std::{
    borrow::Cow,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use bean_types::{Risk, ToolSpec, config::McpServerConfig as ServerConfig};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock},
    service::{RoleClient, RunningService},
    transport::TokioChildProcess,
};
use tokio::{
    process::Command,
    sync::Mutex,
    time::{sleep, timeout},
};

use crate::{ToolCtx, ToolError, ToolRegistry, tool::Tool, untrusted::wrap};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

type ClientService = RunningService<RoleClient, ()>;

#[derive(Debug)]
enum McpCallFailure {
    Timeout,
    Transport(String),
}

/// Các timeout của một kết nối MCP.
#[derive(Debug, Clone, Copy)]
pub struct McpTimeouts {
    /// Timeout spawn + initialize.
    pub connect: Duration,
    /// Timeout discovery toàn bộ trang.
    pub discover: Duration,
    /// Timeout mỗi lần gọi tool.
    pub call: Duration,
    /// Timeout đóng service và tiến trình con.
    pub shutdown: Duration,
    /// Backoff tối thiểu trước khi thử lại một stdio server bị rớt.
    pub reconnect: Duration,
}

impl Default for McpTimeouts {
    fn default() -> Self {
        Self {
            connect: DEFAULT_CONNECT_TIMEOUT,
            discover: DEFAULT_DISCOVERY_TIMEOUT,
            call: DEFAULT_CALL_TIMEOUT,
            shutdown: DEFAULT_SHUTDOWN_TIMEOUT,
            reconnect: DEFAULT_RECONNECT_DELAY,
        }
    }
}

/// Lỗi khi khởi tạo hoặc vận hành một MCP server.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Lỗi có ngữ cảnh của server, không làm lộ biến môi trường/args.
    #[error("MCP server `{server}`: {message}")]
    Server {
        /// Tên server trong cấu hình.
        server: String,
        /// Chi tiết đã lọc secret.
        message: String,
    },
    /// Server không trả initialize trong thời gian cho phép.
    #[error("MCP server `{server}` treo khi khởi tạo ({} ms)", .timeout.as_millis())]
    ConnectTimeout {
        /// Tên server trong cấu hình.
        server: String,
        /// Timeout đã áp dụng.
        timeout: Duration,
    },
    /// `tools/list` treo hoặc trả lỗi.
    #[error("MCP server `{server}` không discovery được tool: {message}")]
    Discovery {
        /// Tên server trong cấu hình.
        server: String,
        /// Lỗi transport/protocol.
        message: String,
    },
    /// Không đóng được service trong thời gian cho phép.
    #[error("MCP server `{server}` không đóng sạch trong {} ms", .timeout.as_millis())]
    ShutdownTimeout {
        /// Tên server trong cấu hình.
        server: String,
        /// Timeout đã áp dụng.
        timeout: Duration,
    },
}

impl McpError {
    fn server(server: &str, message: impl Into<String>) -> Self {
        Self::Server {
            server: server.to_string(),
            message: message.into(),
        }
    }
}

/// Một phần tên biến khiến tên đó trông như mang secret.
///
/// Cùng ý niệm với `SENSITIVE_KEY_PARTS` ở `bean-security/src/audit.rs`; crate này
/// **không** phụ thuộc `bean-security` (chiều phụ thuộc là ngược lại) nên khai báo
/// riêng thay vì tạo phụ thuộc vòng.
const SENSITIVE_NAME_PARTS: &[&str] = &[
    "key",
    "token",
    "secret",
    "password",
    "authorization",
    "credential",
];

fn looks_sensitive(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    SENSITIVE_NAME_PARTS
        .iter()
        .any(|part| lowered.contains(part))
}

/// Cặp `(tên, giá trị)` kế thừa từ môi trường tiến trình Bean cho một MCP server.
///
/// Tách thành hàm thuần để **quy tắc chọn biến** test được mà không cần spawn: phần
/// đọc giá trị thì phụ thuộc môi trường, nhưng quyết định *có truyền biến nào* thì không.
///
/// Vì sao cần: [`connect_service`] gọi `env_clear()` để MCP server không thấy secret
/// của host (mục 15.6), và `env_clear()` xoá luôn `PATH`. Server nào cần `PATH` để
/// chạy (wrapper có shebang `#!/usr/bin/env <interpreter>`) sẽ chết vì vậy — không có
/// `PATH` thì `env` không tìm thấy trình thông dịch và tiến trình con thoát ngay với
/// status 127. `inherit_env` là cách bật lại *đúng những biến cần*, thay vì bỏ
/// `env_clear()`.
fn inherited_env(server: &ServerConfig) -> Vec<(String, String)> {
    let mut resolved = Vec::new();
    for variable in &server.inherit_env {
        let name = variable.trim();
        // `env` tường minh thắng: nó là giá trị người dùng chủ động đặt, không nên bị
        // môi trường của tiến trình âm thầm ghi đè.
        if name.is_empty() || server.env.contains_key(name) {
            continue;
        }
        match std::env::var(name) {
            Ok(value) => {
                if looks_sensitive(name) {
                    // Cảnh báo chứ không chặn: truyền `GITHUB_TOKEN` cho một MCP server
                    // GitHub là việc hợp lệ và có chủ ý. Chặn sẽ cấm hẳn trường hợp đó,
                    // nên thay vào đó làm cho việc đó **thấy được** trong log.
                    tracing::warn!(
                        server = %server.name,
                        variable = %name,
                        "kế thừa biến có tên nhạy cảm cho MCP server; \
                         nếu đây là secret, cân nhắc khai báo qua `env` để kiểm soát rõ ràng"
                    );
                }
                resolved.push((name.to_string(), value));
            }
            Err(_) => {
                // Im lặng thì người dùng không hiểu vì sao server không lên. Đây là
                // nguyên nhân hợp lệ (Bean chạy dưới systemd, `DISPLAY` không tồn tại).
                tracing::warn!(
                    server = %server.name,
                    variable = %name,
                    "inherit_env khai báo biến không tồn tại trong môi trường tiến trình; bỏ qua"
                );
            }
        }
    }
    resolved
}

/// Timeout gọi tool sau khi áp `[[mcp_servers]].call_timeout_seconds`.
///
/// `0` được coi như **không đặt** (giữ mặc định) thay vì thành `Duration::ZERO`, để một
/// `McpServerConfig` dựng tay trong test không vô tình treo mọi tool call. `Config::validate`
/// đã chặn `0` khi đọc từ file; lớp phòng thủ này giữ bất biến ở tầng API công khai.
fn effective_call_timeout(server: &ServerConfig, default: Duration) -> Duration {
    server
        .call_timeout_seconds
        .filter(|seconds| *seconds > 0)
        .map_or(default, |seconds| Duration::from_secs(u64::from(seconds)))
}

/// Kiểm tra dữ liệu spawn từ config trước khi `Command` có thể panic với NUL/`=` sai.
fn validate_spawn_config(server: &ServerConfig) -> Result<(), McpError> {
    if server.command.trim().is_empty() || server.command.contains('\0') {
        return Err(McpError::server(&server.name, "command rỗng hoặc chứa NUL"));
    }
    if server.args.iter().any(|arg| arg.contains('\0')) {
        return Err(McpError::server(
            &server.name,
            "args chứa ký tự NUL không hợp lệ",
        ));
    }
    for (key, value) in &server.env {
        if key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0') {
            return Err(McpError::server(
                &server.name,
                "env có key rỗng, key chứa '=' / NUL, hoặc value chứa NUL",
            ));
        }
    }
    // Lưới an toàn cuối cho `inherit_env`: `Config::validate` đã chặn tên biến sai
    // khi đọc từ file, nhưng `McpServerConfig` cũng có thể dựng tay trong test, và
    // `Command::env` **panic** với tên rỗng — không có cơ hội trả lỗi cho người dùng.
    if server
        .inherit_env
        .iter()
        .any(|name| name.trim().is_empty() || name.contains('=') || name.contains('\0'))
    {
        return Err(McpError::server(
            &server.name,
            "inherit_env có tên biến rỗng hoặc chứa '=' / NUL",
        ));
    }
    Ok(())
}

async fn connect_service(
    server: &ServerConfig,
    timeouts: McpTimeouts,
) -> Result<ClientService, McpError> {
    validate_spawn_config(server)?;
    let mut command = Command::new(&server.command);
    command.args(&server.args).env_clear().kill_on_drop(true);
    // Không kế thừa toàn bộ host env (có thể chứa secret); chỉ truyền `inherit_env`
    // (biến người dùng liệt kê) rồi tới `env` tường minh. Thứ tự này cũng đảm bảo
    // `env` thắng: xem [`inherited_env`].
    for (name, value) in inherited_env(server) {
        command.env(name, value);
    }
    for (key, value) in &server.env {
        command.env(key, value);
    }
    let connecting = async {
        let transport = TokioChildProcess::new(command).map_err(|error| {
            McpError::server(&server.name, format!("không spawn được: {error}"))
        })?;
        let service = ().serve(transport).await.map_err(|error| {
            McpError::server(&server.name, format!("initialize thất bại: {error}"))
        })?;
        Ok::<ClientService, McpError>(service)
    };
    timeout(timeouts.connect, connecting)
        .await
        .map_err(|_| McpError::ConnectTimeout {
            server: server.name.clone(),
            timeout: timeouts.connect,
        })?
}

struct McpConnection {
    server: String,
    config: ServerConfig,
    timeouts: McpTimeouts,
    // `RunningService::call_tool_*` nhận `&self`; Mutex giữ quyền sở hữu khi shutdown
    // và ngăn service bị đóng giữa lúc một tool call đang chạy.
    service: Mutex<Option<ClientService>>,
    reconnect_lock: Mutex<()>,
    reconnect_failures: AtomicU32,
}

impl fmt::Debug for McpConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpConnection")
            .field("server", &self.server)
            .field("trusted", &self.config.trust)
            .field("reconnect_failures", &self.reconnect_failures)
            .finish_non_exhaustive()
    }
}

impl McpConnection {
    async fn connect(server: &ServerConfig, timeouts: McpTimeouts) -> Result<Self, McpError> {
        let service = connect_service(server, timeouts).await?;
        Ok(Self {
            server: server.name.clone(),
            config: server.clone(),
            timeouts,
            service: Mutex::new(Some(service)),
            reconnect_lock: Mutex::new(()),
            reconnect_failures: AtomicU32::new(0),
        })
    }

    async fn reconnect(&self) -> Result<(), McpError> {
        let _guard = self.reconnect_lock.lock().await;
        if let Some(old_service) = self.service.lock().await.take() {
            // Call đang treo có thể không đóng trong shutdown timeout; drop RunningService
            // để TokioChildProcess kill/reap child ngay, rồi spawn instance mới.
            drop(old_service);
        }
        let attempt = self.reconnect_failures.fetch_add(1, Ordering::AcqRel);
        let multiplier = 1_u32 << attempt.min(5);
        let delay = self
            .timeouts
            .reconnect
            .saturating_mul(multiplier)
            .min(MAX_RECONNECT_DELAY);
        if !delay.is_zero() {
            sleep(delay).await;
        }
        let service = connect_service(&self.config, self.timeouts).await?;
        *self.service.lock().await = Some(service);
        self.reconnect_failures.store(0, Ordering::Release);
        Ok(())
    }

    async fn call_tool(
        &self,
        remote_name: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
    ) -> Result<CallToolResponse, McpCallFailure> {
        let params = CallToolRequestParams::new(Cow::Owned(remote_name.to_owned()))
            .with_arguments(arguments);
        let service = self.service.lock().await;
        let Some(service) = service.as_ref() else {
            return Err(McpCallFailure::Transport("kết nối đã đóng".into()));
        };
        match timeout(self.timeouts.call, service.call_tool_once(params)).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(McpCallFailure::Transport(error.to_string())),
            Err(_) => Err(McpCallFailure::Timeout),
        }
    }

    async fn discover(&self) -> Result<Vec<rmcp::model::Tool>, McpError> {
        let service = self.service.lock().await;
        let service = service
            .as_ref()
            .ok_or_else(|| McpError::server(&self.server, "kết nối đã đóng trước khi discovery"))?;
        timeout(self.timeouts.discover, service.list_all_tools())
            .await
            .map_err(|_| McpError::Discovery {
                server: self.server.clone(),
                message: format!(
                    "tools/list treo sau {} ms",
                    self.timeouts.discover.as_millis()
                ),
            })?
            .map_err(|error| McpError::Discovery {
                server: self.server.clone(),
                message: error.to_string(),
            })
    }

    async fn close(&self) -> Result<(), McpError> {
        let Some(mut service) = self.service.lock().await.take() else {
            return Ok(());
        };
        match service.close_with_timeout(self.timeouts.shutdown).await {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(McpError::ShutdownTimeout {
                server: self.server.clone(),
                timeout: self.timeouts.shutdown,
            }),
            Err(error) => Err(McpError::server(
                &self.server,
                format!("task đóng MCP lỗi: {error}"),
            )),
        }
    }
}

/// Quản lý các kết nối MCP cần đóng tường minh khi Bean thoát.
#[derive(Debug, Default)]
pub struct McpRuntime {
    connections: Vec<Arc<McpConnection>>,
}

impl McpRuntime {
    /// Nạp tất cả server; lỗi từng server chỉ log cảnh báo và không chặn agent.
    pub async fn load(servers: &[ServerConfig], registry: &mut ToolRegistry) -> Self {
        Self::load_with_timeouts(servers, registry, McpTimeouts::default()).await
    }

    /// Như [`Self::load`] nhưng dùng timeout tùy biến (chủ yếu cho test).
    pub async fn load_with_timeouts(
        servers: &[ServerConfig],
        registry: &mut ToolRegistry,
        timeouts: McpTimeouts,
    ) -> Self {
        let mut runtime = Self::default();
        for server in servers {
            match runtime.register_server(server, registry, timeouts).await {
                Ok(0) => tracing::warn!(
                    server = %server.name,
                    "MCP server không có tool hợp lệ; bỏ qua"
                ),
                Ok(count) => tracing::info!(
                    server = %server.name,
                    tool_count = count,
                    "đã đăng ký MCP tools"
                ),
                Err(error) => tracing::warn!(
                    server = %server.name,
                    %error,
                    "bỏ qua MCP server lỗi; agent vẫn khởi động"
                ),
            }
        }
        runtime
    }

    /// Kết nối một server, discovery và đăng ký các tool của nó.
    ///
    /// # Errors
    /// Trả lỗi nếu spawn, initialize, discovery hoặc đóng server lỗ.
    pub async fn register_server(
        &mut self,
        server: &ServerConfig,
        registry: &mut ToolRegistry,
        timeouts: McpTimeouts,
    ) -> Result<usize, McpError> {
        // `call_timeout_seconds` ghi đè timeout gọi tool **cho riêng server này**: một
        // server có tool chạy vài chục giây không nên kéo dài timeout của các server
        // khác, và ngược lại server nhanh không cần chờ trace dài.
        let timeouts = McpTimeouts {
            call: effective_call_timeout(server, timeouts.call),
            ..timeouts
        };
        let connection = Arc::new(McpConnection::connect(server, timeouts).await?);
        let definitions = match connection.discover().await {
            Ok(definitions) => definitions,
            Err(error) => {
                let _ = connection.close().await;
                return Err(error);
            }
        };
        let mut registered = 0;
        // (M22) Tag RBAC áp cho **mọi** tool của server này: khai báo ở `[[mcp_servers]]`
        // thay vì gắn tay từng tool (server MCP không tiết lộ tag của tool).
        let required_tags = Arc::new(
            server
                .tool_tags
                .iter()
                .map(|tag| tag.trim().to_string())
                .filter(|tag| !tag.is_empty())
                .collect::<Vec<String>>(),
        );
        for definition in definitions {
            let parameters = definition.schema_as_json_value();
            let remote_name = definition.name.into_owned();
            let exposed_name = format!("mcp__{}__{}", server.name, remote_name);
            let description = definition
                .description
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map_or_else(
                    || format!("Tool `{remote_name}` từ MCP server `{}`.", server.name),
                    ToOwned::to_owned,
                );
            let tool = Arc::new(McpTool {
                connection: connection.clone(),
                remote_name,
                spec: ToolSpec::new(exposed_name, description, parameters),
                trusted: server.trust,
                required_tags: required_tags.clone(),
            });
            match registry.register(tool) {
                Ok(()) => registered += 1,
                Err(error) => tracing::warn!(
                    server = %server.name,
                    %error,
                    "bỏ qua MCP tool trùng tên"
                ),
            }
        }
        if registered == 0 {
            let _ = connection.close().await;
            return Ok(0);
        }
        self.connections.push(connection);
        Ok(registered)
    }

    /// Đóng tất cả connection; an toàn gọi nhiều lần.
    pub async fn close(&self) {
        for connection in &self.connections {
            if let Err(error) = connection.close().await {
                tracing::warn!(server = %connection.server, %error, "không đóng sạch MCP server");
            }
        }
    }
}

/// Adapter từ một MCP tool definition sang `bean_tools::Tool`.
struct McpTool {
    connection: Arc<McpConnection>,
    remote_name: String,
    spec: ToolSpec,
    trusted: bool,
    /// Tag RBAC kế thừa từ `[[mcp_servers]].tool_tags` (M22).
    ///
    /// Dùng `Vec<String>` (không phải `&'static str`) vì tag đến từ file cấu hình chạy
    /// được, không phải literal trong mã.
    required_tags: Arc<Vec<String>>,
}

const fn risk_for_trust(trusted: bool) -> Risk {
    if trusted { Risk::Safe } else { Risk::Confirm }
}

/// Kết quả tool có khối text **đọc được** không.
///
/// `false` nghĩa là server trả về im lặng: mọi khối text đều rỗng hoặc chỉ toàn khoảng
/// trắng. Đây là tín hiệu duy nhất phân biệt được "tool lỗi" với "tool lỗi nhưng server
/// nuốt mất lý do" — xem [`McpTool::silent_remote_error`].
///
/// Ảnh/âm thanh/resource vẫn tính là **có** nội dung (model nhìn được, mục 26); chỉ
/// text rỗng mới bị coi là im lặng.
fn has_readable_text(content: &[ContentBlock]) -> bool {
    content.iter().any(|block| match block {
        ContentBlock::Text(text) => !text.text.trim().is_empty(),
        _ => true,
    })
}

/// Payload thay thế khi MCP server báo lỗi mà không kèm message.
///
/// Giữ nguyên `raw` để không đánh mất thông tin, và nói rõ cho model biết phải làm gì:
/// đây là lỗi *cục bộ của server*, không phải hành động bị từ chối — nên cách sửa đúng là
/// kiểm tra tham số, đặc biệt là đường dẫn phải **tuyệt đối** cho tool ngoài core.
fn silent_error_payload(server: &str, raw: &CallToolResult) -> serde_json::Value {
    serde_json::json!({
        "isError": true,
        "error": format!(
            "MCP server `{server}` báo tool thất bại nhưng KHÔNG kèm message lỗi. \
             Server bỏ sót chi tiết ở chế độ rút gọn (ví dụ chrome-devtools-mcp với `--slim`). \
             Hãy kiểm tra lại tham số; với `url`/`file` phải dùng đường dẫn TUYỆT ĐỐI trên máy chủ \
             (xem mục `# Environment` của system prompt), đừng suy ra từ đường dẫn tương đối."
        ),
        "raw": raw,
    })
}

impl McpTool {
    fn mark_untrusted(ctx: &ToolCtx) {
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn remote_error(ctx: &ToolCtx, payload: serde_json::Value) -> ToolError {
        Self::mark_untrusted(ctx);
        let encoded = serde_json::to_string(&payload)
            .unwrap_or_else(|_| "{\"error\":\"không serialize được kết quả MCP\"}".to_string());
        ToolError::Mcp(wrap(&encoded))
    }

    /// `isError: true` mà **không kèm message lỗi** — server nuốt mất lý do.
    ///
    /// Xảy ra thật với `chrome-devtools-mcp` ở chế độ `--slim` (đã bật trong `bean.toml`):
    /// `SlimMcpResponse.handle()` chỉ serialize `responseLines` — rỗng khi handler ném lỗi —
    /// và **không** kèm message, trong khi `McpResponse.handle()` ở chế độ thường có
    /// `errorMessage: this.#error?.message`. Đã kiểm tra cả bản 1.10.1: y hệt, chưa sửa.
    ///
    /// Trả về đúng thông báo lỗi cho model thay vì chuỗi rỗng — nếu không, model và UI chỉ
    /// thấy `{"content":[{"type":"text","text":""}],"isError":true}` và không có cách nào
    /// đoán nguyên nhân.
    fn silent_remote_error(ctx: &ToolCtx, server: &str, raw: &CallToolResult) -> ToolError {
        Self::mark_untrusted(ctx);
        let encoded =
            serde_json::to_string(&silent_error_payload(server, raw)).unwrap_or_else(|_| {
                "{\"isError\":true,\"error\":\"MCP server báo lỗi mà không kèm message\"}"
                    .to_string()
            });
        ToolError::Mcp(wrap(&encoded))
    }

    fn call_failure(&self, failure: McpCallFailure) -> ToolError {
        let detail = match failure {
            McpCallFailure::Timeout => format!(
                "MCP tool `{}` treo sau {} ms",
                self.spec.name,
                self.connection.timeouts.call.as_millis()
            ),
            McpCallFailure::Transport(message) => {
                format!("MCP tool `{}` lỗi transport: {message}", self.spec.name)
            }
        };
        ToolError::Mcp(detail)
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        risk_for_trust(self.trusted)
    }

    fn describe(&self, _args: &serde_json::Value) -> String {
        format!(
            "MCP server `{}`, tool `{}`",
            self.connection.server, self.remote_name
        )
    }

    /// MCP luôn là nguồn ngoài lõi (mục 15.4/16): kết quả **và cả lỗi** đều đã bọc
    /// `<untrusted_content>` trong `call`, nên khai báo `true` để agent loop bật cờ
    /// ngay cả khi `call` trả `Err` trước khi tới chỗ bọc.
    fn marks_untrusted(&self) -> bool {
        true
    }

    /// (M22) Tag RBAC kế thừa từ `[[mcp_servers]].tool_tags`.
    ///
    /// Rỗng ⇒ mọi role đã cấp quyền đều thấy (giữ hành vi cũ cho server không gắn tag).
    fn required_tags(&self) -> Vec<&str> {
        self.required_tags.iter().map(String::as_str).collect()
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let serde_json::Value::Object(arguments) = args else {
            return Err(ToolError::InvalidArgs(format!(
                "arguments phải là JSON object theo schema: {}",
                self.spec.parameters
            )));
        };
        let mut retried = false;
        let response = loop {
            match self
                .connection
                .call_tool(&self.remote_name, arguments.clone())
                .await
            {
                Ok(response) => break response,
                Err(failure) if !retried => {
                    retried = true;
                    tracing::warn!(
                        server = %self.connection.server,
                        tool = %self.spec.name,
                        ?failure,
                        "MCP transport rớt; thực hiện reconnect một lần"
                    );
                    if let Err(error) = self.connection.reconnect().await {
                        return Err(ToolError::Mcp(format!(
                            "MCP server `{}` reconnect thất bại: {error}",
                            self.connection.server
                        )));
                    }
                }
                Err(failure) => return Err(self.call_failure(failure)),
            }
        };

        match response {
            CallToolResponse::Complete(result) => {
                let is_error = result.is_error.unwrap_or(false);
                // Server báo lỗi nhưng không kèm lý do: phải nói rõ thay vì trả chuỗi
                // rỗng (xem `silent_remote_error`).
                if is_error && !has_readable_text(&result.content) {
                    return Err(Self::silent_remote_error(
                        ctx,
                        &self.connection.server,
                        &result,
                    ));
                }
                let encoded = match serde_json::to_string(&result) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        return Err(Self::remote_error(
                            ctx,
                            serde_json::json!({ "error": error.to_string() }),
                        ));
                    }
                };
                Self::mark_untrusted(ctx);
                let wrapped = wrap(&encoded);
                if is_error {
                    Err(ToolError::Mcp(wrapped))
                } else {
                    Ok(wrapped)
                }
            }
            CallToolResponse::InputRequired(_) => Err(Self::remote_error(
                ctx,
                serde_json::json!({
                    "error": "MCP server yêu cầu input tương tác; Bean chưa hỗ trợ MRTR input_required"
                }),
            )),
            CallToolResponse::Task(_) => Err(Self::remote_error(
                ctx,
                serde_json::json!({
                    "error": "MCP server trả task handle; Bean chưa hỗ trợ task polling"
                }),
            )),
            _ => Err(Self::remote_error(
                ctx,
                serde_json::json!({ "error": "MCP server trả loại kết quả không hỗ trợ" }),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeMap;

    use super::*;

    /// Dựng `CallToolResult` từ đúng JSON server gửi trên wire.
    ///
    /// `CallToolResult` là `#[non_exhaustive]` nên không dựng bằng struct literal được;
    /// deserialize từ JSON vừa khả thi vừa sát thực tế hơn.
    fn result_from_wire(json: serde_json::Value) -> CallToolResult {
        serde_json::from_value(json).expect("JSON kết quả MCP phải hợp lệ")
    }

    /// Hồi quy đúng lỗi gặp thật: `chrome-devtools-mcp` `--slim` trả
    /// `{"content":[{"type":"text","text":""}],"isError":true}` — không có lý do.
    #[test]
    fn empty_text_block_reads_as_silent_failure() {
        let result = result_from_wire(serde_json::json!({
            "content": [{ "type": "text", "text": "" }],
            "isError": true
        }));
        assert!(!has_readable_text(&result.content));
    }

    #[test]
    fn whitespace_only_text_is_also_silent() {
        let result = result_from_wire(serde_json::json!({
            "content": [{ "type": "text", "text": "   \n\t  " }],
            "isError": true
        }));
        assert!(
            !has_readable_text(&result.content),
            "chỉ khoảng trắng thì model cũng không đọc được gì"
        );
    }

    /// Có message thật thì phải giữ nguyên đường cũ — không được đụng vào kết quả hợp lệ.
    #[test]
    fn real_message_still_counts_as_readable() {
        let result = result_from_wire(serde_json::json!({
            "content": [{ "type": "text", "text": "Navigated to file:///tmp/a.html." }],
            "isError": false
        }));
        assert!(has_readable_text(&result.content));
    }

    /// Server trả về cả ảnh (mục 26) thì vẫn là nội dung đọc được, không phải im lặng.
    #[test]
    fn image_block_counts_as_readable_content() {
        let result = result_from_wire(serde_json::json!({
            "content": [
                { "type": "text", "text": "" },
                { "type": "image", "data": "aGVsbG8=", "mimeType": "image/png" }
            ],
            "isError": true
        }));
        assert!(has_readable_text(&result.content));
    }

    #[test]
    fn no_content_at_all_is_silent() {
        let result = result_from_wire(serde_json::json!({ "isError": true }));
        assert!(!has_readable_text(&result.content));
    }

    /// Payload thay thế phải **có nội dung dùng được**: nêu tên server và chỉ cách sửa.
    ///
    /// Trước khi có `silent_error_payload`, model nhận đúng `text: ""` — nên đây là
    /// hồi quy cho đúng cái lỗi người dùng gặp phải.
    #[test]
    fn silent_error_payload_explains_the_cause_and_keeps_raw() {
        let raw = result_from_wire(serde_json::json!({
            "content": [{ "type": "text", "text": "" }],
            "isError": true
        }));
        let payload = silent_error_payload("chrome", &raw);

        let message = payload["error"].as_str().expect("phải có error");
        assert!(message.contains("chrome"), "phải nêu tên server: {message}");
        assert!(
            message.contains("TUYỆT ĐỐI"),
            "phải chỉ cách sửa đúng: {message}"
        );

        // `raw` giữ lại nguyên trạng để không mất thông tin gì của server.
        assert_eq!(payload["raw"]["content"][0]["text"], "");
        assert_eq!(payload["isError"], true);
    }

    #[test]
    fn risk_is_confirm_unless_server_is_trusted() {
        assert_eq!(risk_for_trust(false), Risk::Confirm);
        assert_eq!(risk_for_trust(true), Risk::Safe);
    }

    fn fixture() -> ServerConfig {
        ServerConfig {
            name: "fixture".to_string(),
            command: "/bin/true".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            trust: false,
            tool_tags: Vec::new(),
            call_timeout_seconds: None,
            inherit_env: Vec::new(),
        }
    }

    #[test]
    fn call_timeout_defaults_unless_configured() {
        let mut server = fixture();
        let default = Duration::from_secs(60);
        assert_eq!(effective_call_timeout(&server, default), default);

        server.call_timeout_seconds = Some(300);
        assert_eq!(
            effective_call_timeout(&server, default),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn zero_call_timeout_falls_back_instead_of_failing_every_call() {
        // `Config::validate` chặn 0, nhưng struct dựng tay thì không: `Duration::ZERO`
        // sẽ khiến mọi tool call treo tức thì, nên phải rơi về mặc định.
        let mut server = fixture();
        server.call_timeout_seconds = Some(0);
        let default = Duration::from_secs(60);
        assert_eq!(effective_call_timeout(&server, default), default);
    }

    #[test]
    fn inherit_env_skips_names_shadowed_by_explicit_env() {
        // `env` tường minh phải thắng — kiểm tra ở mức hàm thuần, không cần spawn.
        let mut server = fixture();
        server
            .env
            .insert("PATH".to_string(), "tu-config".to_string());
        server.inherit_env = vec!["PATH".to_string()];
        assert!(
            inherited_env(&server).is_empty(),
            "biến có trong `env` không được kế thừa lần nữa"
        );
    }

    #[test]
    fn inherit_env_ignores_blank_and_missing_variables() {
        let mut server = fixture();
        server.inherit_env = vec!["  ".to_string(), "BEAN_BIEN_KHONG_TON_TAI_XYZ".to_string()];
        assert!(inherited_env(&server).is_empty());
    }

    #[test]
    fn inherit_env_returns_existing_variable_value() {
        // `PATH` gần như luôn tồn tại; đây là biến mà `env_clear()` xoá đi.
        let mut server = fixture();
        server.inherit_env = vec!["PATH".to_string()];
        let Some(expected) = std::env::var("PATH").ok() else {
            return;
        };
        let resolved = inherited_env(&server);
        assert_eq!(resolved.len(), 1, "{resolved:?}");
        assert_eq!(resolved[0], ("PATH".to_string(), expected));
    }

    #[test]
    fn sensitive_names_are_flagged_for_the_operator() {
        // Cảnh báo, không chặn: truyền GITHUB_TOKEN cho MCP server GitHub là hợp lệ.
        for name in [
            "GITHUB_TOKEN",
            "OPENAI_API_KEY",
            "my_password",
            "AWS_SECRET_ACCESS_KEY",
            "DB_CREDENTIAL",
            "auth_authorization",
        ] {
            assert!(looks_sensitive(name), "{name} phải bị cảnh báo");
        }
        for name in ["PATH", "HOME", "DISPLAY", "LANG", "USER", "XDG_RUNTIME_DIR"] {
            assert!(!looks_sensitive(name), "{name} không phải secret");
        }
    }

    #[test]
    fn invalid_inherit_env_names_are_rejected_before_spawn() {
        // Lưới an toàn cuối: chặn ở đây thay vì để `Command::env` panic.
        for bad in ["", "  ", "A=B", "HAS\0NUL"] {
            let mut server = fixture();
            server.inherit_env = vec![bad.to_string()];
            // Chỉ dùng `assert!`: workspace cấm cả `panic!` lẫn `unwrap_used` trong
            // unit test nằm trong `src/` (khác với test tích hợp trong `tests/`).
            let rejected = match validate_spawn_config(&server) {
                Err(error) => error.to_string().contains("inherit_env"),
                Ok(()) => false,
            };
            assert!(rejected, "inherit_env = {bad:?} phải bị chặn");
        }
    }
}
