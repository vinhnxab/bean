//! Tool `run_shell` (agents.md mục 7.3, 15.2).
//!
//! Chạy trong [`crate::sandbox::Sandbox`]: mặc định docker với workspace mount,
//! `--network none`, `--cap-drop ALL`, no-new-privileges; chế độ host ⇒ mức rủi ro
//! `Dangerous`. Mức rủi ro **cơ bản** lấy từ [`Sandbox::base_risk`]; deny-list mẫu
//! và "cho phép trong phiên" do [`crate::policy::Policy`] áp thêm ở agent loop.

use std::sync::Arc;

use beanagent_tools::{Tool, ToolError, TypedTool};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::sandbox::{Sandbox, ShellOutcome};

/// Tham số của `run_shell`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunShellParams {
    /// Câu lệnh shell sẽ chạy trong sandbox (mặc định `sh -c`, cwd = `/workspace`).
    /// Ví dụ: `ls -la` hoặc `python3 script.py` (nếu image có sẵn).
    pub command: String,
}

/// Dựng tool `run_shell` gắn với một sandbox.
#[must_use]
pub fn run_shell(sandbox: Arc<Sandbox>) -> Arc<dyn Tool> {
    let base_risk = sandbox.base_risk();
    Arc::new(TypedTool::new(
        "run_shell",
        base_risk,
        move |_ctx: &beanagent_tools::ToolCtx, p: RunShellParams| {
            let sandbox = sandbox.clone();
            let cancel = _ctx.cancel.clone();
            async move {
                if p.command.trim().is_empty() {
                    return Err(ToolError::InvalidArgs(
                        "command rỗng — cần một câu lệnh để chạy".to_string(),
                    ));
                }
                // Sandbox có timeout riêng theo cấu hình để kill container khi hết giờ;
                // agent loop cũng bọc `tool_timeout_seconds` bên ngoài (mục 6).
                let timeout = std::time::Duration::from_secs(sandbox.timeout_seconds());
                let outcome = sandbox
                    .run(&p.command, timeout, &cancel)
                    .await
                    .map_err(|e| ToolError::Io(e.to_string()))?;
                Ok(format_outcome(&outcome))
            }
        },
    ))
}

/// Định dạng output gửi lại cho model: exit code + stdout + stderr (đã cắt bên trong
/// sandbox ở ranh giới UTF-8).
fn format_outcome(outcome: &ShellOutcome) -> String {
    let code = outcome
        .exit_code
        .map_or_else(|| "không có (bị kill)".to_string(), |c| c.to_string());
    let mut out = format!("exit code: {code}");
    if !outcome.stdout.is_empty() {
        out.push_str("\n--- stdout ---\n");
        out.push_str(&outcome.stdout);
    }
    if !outcome.stderr.is_empty() {
        out.push_str("\n--- stderr ---\n");
        out.push_str(&outcome.stderr);
    }
    if outcome.stdout.is_empty() && outcome.stderr.is_empty() {
        out.push_str("\n(không có output)");
    }
    out
}

/// Chỉ dùng cho test risk của tool ở đây (không cần docker).
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use beanagent_types::Risk;
    use beanagent_types::config::{SandboxConfig, SandboxMode};

    #[test]
    fn risk_follows_sandbox_mode() {
        let ws = tempfile::tempdir().unwrap();
        let docker = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Docker,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let host = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Host,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let args = serde_json::json!({"command": "ls"});
        let docker_tool = run_shell(Arc::new(docker));
        let host_tool = run_shell(Arc::new(host));
        assert_eq!(docker_tool.risk(&args), Risk::Confirm);
        assert_eq!(host_tool.risk(&args), Risk::Dangerous);
    }

    #[test]
    fn spec_has_description_and_command_param() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(SandboxConfig::default(), ws.path().to_path_buf());
        let tool = run_shell(Arc::new(sandbox));
        let spec = tool.spec();
        assert_eq!(spec.name, "run_shell");
        assert!(!spec.description.is_empty());
        assert!(spec.parameters.get("properties").is_some());
        assert!(spec.parameters.get("required").is_some());
    }
}
