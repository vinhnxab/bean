//! Section `[tools]`, `[security.sandbox]`, `[security]`.

use serde::{Deserialize, Serialize};

use super::*;

/// `[tools]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// Nhóm tool bật lên (xem [`KNOWN_TOOL_GROUPS`]).
    pub enabled: Vec<String>,
    /// Cấu hình riêng cho `web_search`.
    pub web_search: WebSearchConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: KNOWN_TOOL_GROUPS.iter().map(|s| (*s).to_string()).collect(),
            web_search: WebSearchConfig::default(),
        }
    }
}

/// `[security.sandbox]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxConfig {
    /// `docker` (mặc định) hoặc `host`.
    pub mode: SandboxMode,
    /// Image sandbox (Debian slim, **không** kèm Node — agents.md mục 15.2).
    pub image: String,
    /// Cho container ra mạng hay không (mặc định: không).
    pub network: bool,
    /// Trần bộ nhớ (`docker --memory`).
    pub memory: String,
    /// Số CPU được dùng.
    pub cpus: f32,
    /// Trần số tiến trình trong container (`docker --pids-limit`, chống fork bomb).
    pub pids_limit: u32,
    /// Thời gian tối đa cho một lệnh shell (giây).
    pub timeout_seconds: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::Docker,
            image: "Bean-sandbox:latest".to_string(),
            network: false,
            memory: "512m".to_string(),
            cpus: 1.0,
            pids_limit: 64,
            timeout_seconds: 60,
        }
    }
}

/// `[security]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    /// Ngân sách token mỗi ngày (mục 15.9).
    pub daily_token_budget: u64,
    /// Trần thời gian cho mỗi tool call (giây, agents.md mục 6).
    pub tool_timeout_seconds: u64,
    /// Cấu hình sandbox cho `run_shell`.
    pub sandbox: SandboxConfig,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            daily_token_budget: 2_000_000,
            tool_timeout_seconds: 60,
            sandbox: SandboxConfig::default(),
        }
    }
}
