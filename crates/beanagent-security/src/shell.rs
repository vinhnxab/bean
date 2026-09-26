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
    run_shell_for_projects(sandbox, Default::default())
}

/// Dựng tool `run_shell` với sandbox riêng cho từng project profile (M21.1).
///
/// `by_project` tra theo [`ToolCtx::project`]; project không có trong map thì dùng
/// `sandbox` mặc định. Nhờ vậy lệnh trong project A **không** nhìn thấy (hay ghi được)
/// file của project B — giữ đúng nguyên tắc path jail mục 15.1 khi có nhiều project.
#[must_use]
pub fn run_shell_for_projects(
    sandbox: Arc<Sandbox>,
    by_project: std::collections::BTreeMap<String, Arc<Sandbox>>,
) -> Arc<dyn Tool> {
    // Mức rủi ro cơ bản lấy từ sandbox **mặc định**; mọi sandbox đều dùng chung
    // `[security.sandbox]` nên cùng chế độ docker/host.
    let base_risk = sandbox.base_risk();
    Arc::new(
        TypedTool::new(
            "run_shell",
            base_risk,
            move |ctx: &beanagent_tools::ToolCtx, p: RunShellParams| {
                let sandbox = by_project
                    .get(&ctx.project)
                    .cloned()
                    .unwrap_or_else(|| sandbox.clone());
                let cancel = ctx.cancel.clone();
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
        )
        // Mục 15.4: stdout/stderr là dữ liệu ngoài lõi (lệnh trong sandbox có thể in ra
        // nội dung file do kẻ tấn công kiểm soát) — bọc thẻ + bật cờ `untrusted_seen`.
        .untrusted()
        // RBAC (M21.6): lệnh shell **luôn** có thể sửa code, nên phải cùng bị chặn với
        // `dev-write` — nếu chỉ chặn `write_file`/`edit_file` thì `qa` chạy
        // `echo x > src/lib.rs` là lách được four-eyes. Tag `infra-scan` cho phép
        // security-scan chạy `nmap`/`trivy` (M23) mà không cần quyền ghi code.
        .requires_tags(["dev-write", "infra-scan"]),
    )
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
