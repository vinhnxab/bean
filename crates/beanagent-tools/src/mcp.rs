//! MCP client stdio tích hợp BeanAgent (agents.md mục 16).
//!
//! Mỗi server được spawn thành tiến trình con, handshake bằng `rmcp`, discovery qua
//! `tools/list`, rồi đăng ký từng tool thành `mcp__<server>__<tool>`. Server lỗi hoặc treo
//! chỉ bị log và bỏ qua; registry built-in vẫn dùng được để agent khởi động.

use std::{borrow::Cow, sync::Arc, time::Duration};

use async_trait::async_trait;
use beanagent_types::{Risk, ToolSpec, config::McpServerConfig as ServerConfig};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResponse},
    service::{RoleClient, RunningService},
    transport::TokioChildProcess,
};
use tokio::{process::Command, sync::Mutex, time::timeout};

use crate::{ToolCtx, ToolError, ToolRegistry, tool::Tool, untrusted::wrap};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

type ClientService = RunningService<RoleClient, ()>;

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
}

impl Default for McpTimeouts {
    fn default() -> Self {
        Self {
            connect: DEFAULT_CONNECT_TIMEOUT,
            discover: DEFAULT_DISCOVERY_TIMEOUT,
            call: DEFAULT_CALL_TIMEOUT,
            shutdown: DEFAULT_SHUTDOWN_TIMEOUT,
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
    Ok(())
}

/// Một kết nối stdio tới server, dùng chung cho các tool của server đó.
#[derive(Debug)]
struct McpConnection {
    server: String,
    timeouts: McpTimeouts,
    // `RunningService::call_tool_*` nhận `&self`; Mutex giữ quyền sở hữu khi shutdown
    // và ngăn service bị đóng giữa lúc một tool call đang chạy.
    service: Mutex<Option<ClientService>>,
}

impl McpConnection {
    async fn connect(server: &ServerConfig, timeouts: McpTimeouts) -> Result<Self, McpError> {
        validate_spawn_config(server)?;
        let mut command = Command::new(&server.command);
        command.args(&server.args).env_clear().kill_on_drop(true);
        // Không kế thừa toàn bộ host env (có thể chứa secret); chỉ truyền env khai báo.
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

        let service = timeout(timeouts.connect, connecting).await.map_err(|_| {
            McpError::ConnectTimeout {
                server: server.name.clone(),
                timeout: timeouts.connect,
            }
        })??;
        Ok(Self {
            server: server.name.clone(),
            timeouts,
            service: Mutex::new(Some(service)),
        })
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

/// Quản lý các kết nối MCP cần đóng tường minh khi BeanAgent thoát.
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
        let connection = Arc::new(McpConnection::connect(server, timeouts).await?);
        let definitions = match connection.discover().await {
            Ok(definitions) => definitions,
            Err(error) => {
                let _ = connection.close().await;
                return Err(error);
            }
        };
        let mut registered = 0;
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

/// Adapter từ một MCP tool definition sang `beanagent_tools::Tool`.
struct McpTool {
    connection: Arc<McpConnection>,
    remote_name: String,
    spec: ToolSpec,
    trusted: bool,
}

const fn risk_for_trust(trusted: bool) -> Risk {
    if trusted { Risk::Safe } else { Risk::Confirm }
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

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let serde_json::Value::Object(arguments) = args else {
            return Err(ToolError::InvalidArgs(format!(
                "arguments phải là JSON object theo schema: {}",
                self.spec.parameters
            )));
        };
        let params = CallToolRequestParams::new(Cow::Owned(self.remote_name.clone()))
            .with_arguments(arguments);
        let response = {
            let service = self.connection.service.lock().await;
            let Some(service) = service.as_ref() else {
                return Err(ToolError::Mcp(format!(
                    "MCP server `{}` đã đóng",
                    self.connection.server
                )));
            };
            timeout(
                self.connection.timeouts.call,
                service.call_tool_once(params),
            )
            .await
        };

        match response {
            Err(_) => Err(ToolError::Mcp(format!(
                "MCP tool `{}` treo sau {} ms",
                self.spec.name,
                self.connection.timeouts.call.as_millis()
            ))),
            Ok(Err(error)) => Err(Self::remote_error(
                ctx,
                serde_json::json!({ "error": error.to_string() }),
            )),
            Ok(Ok(CallToolResponse::Complete(result))) => {
                let is_error = result.is_error.unwrap_or(false);
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
            Ok(Ok(CallToolResponse::InputRequired(_))) => Err(Self::remote_error(
                ctx,
                serde_json::json!({
                    "error": "MCP server yêu cầu input tương tác; BeanAgent chưa hỗ trợ MRTR input_required"
                }),
            )),
            Ok(Ok(CallToolResponse::Task(_))) => Err(Self::remote_error(
                ctx,
                serde_json::json!({
                    "error": "MCP server trả task handle; BeanAgent chưa hỗ trợ task polling"
                }),
            )),
            Ok(Ok(_)) => Err(Self::remote_error(
                ctx,
                serde_json::json!({ "error": "MCP server trả loại kết quả không hỗ trợ" }),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_is_confirm_unless_server_is_trusted() {
        assert_eq!(risk_for_trust(false), Risk::Confirm);
        assert_eq!(risk_for_trust(true), Risk::Safe);
    }
}
