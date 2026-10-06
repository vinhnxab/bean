//! `McpRuntime`: quan ly nhieu server MCP + dang ky tool vao registry.

use std::sync::Arc;

use bean_types::{ToolSpec, config::McpServerConfig as ServerConfig};

use super::config::{McpError, McpTimeouts, effective_call_timeout};
use super::connection::McpConnection;
use super::tool::McpTool;
use crate::ToolRegistry;

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
            let tool = Arc::new(McpTool::new(
                connection.clone(),
                remote_name,
                ToolSpec::new(exposed_name, description, parameters),
                server.trust,
                required_tags.clone(),
            ));
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
                tracing::warn!(server = %connection.server_name(), %error, "không đóng sạch MCP server");
            }
        }
    }
}
