//! Mot ket noi MCP: spawn, handshake, discovery, goi tool, dong sach.

use std::{
    borrow::Cow,
    fmt,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use bean_types::config::McpServerConfig as ServerConfig;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResponse},
    transport::TokioChildProcess,
};
use tokio::{
    process::Command,
    sync::Mutex,
    time::{sleep, timeout},
};

use super::config::{
    ClientService, McpCallFailure, McpError, McpTimeouts, inherited_env, validate_spawn_config,
};

const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

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

pub(crate) struct McpConnection {
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
    pub(crate) async fn connect(
        server: &ServerConfig,
        timeouts: McpTimeouts,
    ) -> Result<Self, McpError> {
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

    pub(crate) async fn reconnect(&self) -> Result<(), McpError> {
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

    pub(crate) async fn call_tool(
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

    pub(crate) async fn discover(&self) -> Result<Vec<rmcp::model::Tool>, McpError> {
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

    pub(crate) async fn close(&self) -> Result<(), McpError> {
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

impl McpConnection {
    pub(crate) fn server_name(&self) -> &str {
        &self.server
    }
    pub(crate) fn call_timeout(&self) -> Duration {
        self.timeouts.call
    }
}
