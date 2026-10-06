//! MCP config: timeout, loi, sanitize env, validate spawn (thuan, khong I/O).

use std::time::Duration;

use bean_types::config::McpServerConfig as ServerConfig;
use rmcp::service::{RoleClient, RunningService};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RECONNECT_DELAY: Duration = Duration::from_secs(1);

pub(crate) type ClientService = RunningService<RoleClient, ()>;

#[derive(Debug)]
pub(crate) enum McpCallFailure {
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
    pub(crate) fn server(server: &str, message: impl Into<String>) -> Self {
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

pub(crate) fn looks_sensitive(name: &str) -> bool {
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
/// Vì sao cần: `connect_service()` gọi `env_clear()` để MCP server không thấy secret
/// của host (mục 15.6), và `env_clear()` xoá luôn `PATH`. Server nào cần `PATH` để
/// chạy (wrapper có shebang `#!/usr/bin/env <interpreter>`) sẽ chết vì vậy — không có
/// `PATH` thì `env` không tìm thấy trình thông dịch và tiến trình con thoát ngay với
/// status 127. `inherit_env` là cách bật lại *đúng những biến cần*, thay vì bỏ
/// `env_clear()`.
pub(crate) fn inherited_env(server: &ServerConfig) -> Vec<(String, String)> {
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
pub(crate) fn effective_call_timeout(server: &ServerConfig, default: Duration) -> Duration {
    server
        .call_timeout_seconds
        .filter(|seconds| *seconds > 0)
        .map_or(default, |seconds| Duration::from_secs(u64::from(seconds)))
}

/// Kiểm tra dữ liệu spawn từ config trước khi `Command` có thể panic với NUL/`=` sai.
pub(crate) fn validate_spawn_config(server: &ServerConfig) -> Result<(), McpError> {
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
