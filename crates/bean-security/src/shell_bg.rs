//! Tool shell nền (background) — `run_shell_bg`, `shell_bg_output`, `shell_bg_kill`.
//!
//! Vì sao cần: `run_shell` có timeout (mặc định 60s) và **chặn** run cho tới khi
//! lệnh xong — build release ~6 phút hay test suite dài buộc cả lượt đứng chờ, hoặc
//! tệ hơn là bị timeout kill giữa chừng. Bộ tool này cho model khởi động lệnh dài
//! rồi quay lại đọc output theo nhu cầu — cùng cách Claude Code làm với background
//! bash.
//!
//! Thiết kế:
//!
//! * Bảng job [`BgJobs`] dùng chung cho cả ba tool, inject lúc dựng (DIP) — state
//!   sống trong tiến trình, **không** bền qua restart (restart ⇒ job chết theo
//!   tiến trình; lịch sử hội thoại vẫn ghi lại việc khởi động/đọc).
//! * Spawn đi qua cùng [`Sandbox`] như `run_shell`: docker mode giữ nguyên bộ cờ
//!   cách ly (`docker_run_args`, container có tên để kill gọn); host mode giữ
//!   nguyên whitelist (Q8) — risk tính lại **mỗi call** từ watch cell (D22.5b).
//! * `run_shell_bg` trả ngay job id; output gom vào bộ đệm trần 256 KB mỗi stream
//!   (bỏ đầu giữ đuôi); `shell_bg_output` cắt theo ranh giới ký tự UTF-8.
//! * RBAC: cùng `requires_tags(["dev-write", "infra-scan"])` với `run_shell` —
//!   nếu chỉ chặn `run_shell` thì role `qa` lách four-eyes qua chạy nền (M21.6).
//! * Trần hoạt động: tối đa [`MAX_RUNNING`] job chạy đồng thời, mỗi job tối đa
//!   [`MAX_AGE_SECS`] giây (job quên kill bị dừng lười lúc đọc/gọi kế tiếp).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bean_tools::{Tool, ToolError, TypedTool};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, watch};

use crate::sandbox::{Sandbox, host_shell_args, kill_container};
use crate::shell::host_allowlisted;
use bean_types::Risk;
use bean_types::config::SandboxMode;

/// Trần số job nền được chạy đồng thời.
const MAX_RUNNING: usize = 16;
/// Trần số job (kể cả đã xong) giữ trong bảng — bớt bớt job cũ đã kết thúc.
const MAX_HISTORY: usize = 64;
/// Tuổi thọ tối đa của một job — quá hạn bị kill (thực thi lười).
const MAX_AGE_SECS: u64 = 3600;
/// Trần byte mỗi stream (stdout/stderr) giữ trong bộ đệm — bỏ đầu giữ đuôi.
const MAX_STREAM_BYTES: usize = 256 * 1024;
/// Trần ký tự mỗi stream khi trả output cho model.
const OUTPUT_MAX_CHARS: usize = 8_000;

// ---------------------------------------------------------------------------
// Bảng job dùng chung
// ---------------------------------------------------------------------------

/// Bộ đệm output của một stream — trần [`MAX_STREAM_BYTES`], bỏ đầu giữ đuôi.
#[derive(Default)]
struct StreamBuf {
    bytes: Vec<u8>,
    dropped_head: u64,
}

impl StreamBuf {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > MAX_STREAM_BYTES {
            let excess = self.bytes.len() - MAX_STREAM_BYTES;
            self.bytes.drain(..excess);
            self.dropped_head += excess as u64;
        }
    }
}

/// Trạng thái job.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JobState {
    Running,
    Exited(Option<i32>),
    Killed,
    Failed(String),
}

/// Một job nền đã khởi động.
struct BgJob {
    id: String,
    started_at: Instant,
    stdout: Arc<Mutex<StreamBuf>>,
    stderr: Arc<Mutex<StreamBuf>>,
    state: Mutex<JobState>,
    /// `Some` khi còn đang chạy — `finish_watch`/kill lấy ra để `wait()`/`kill()`.
    child: Mutex<Option<tokio::process::Child>>,
    /// Tên container khi docker mode — kill phải `docker kill` vì chỉ kill CLI
    /// client không dừng được container.
    container: Option<String>,
}

/// Bảng job nền dùng chung giữa ba tool — inject lúc dựng tool (DIP).
#[derive(Default)]
pub struct BgJobs {
    next_id: AtomicU64,
    jobs: Mutex<Vec<Arc<BgJob>>>,
}

impl BgJobs {
    async fn start(&self, sandbox: &Sandbox, command: &str) -> Result<Arc<BgJob>, ToolError> {
        let mut all = self.jobs.lock().await;
        // Dọn lịch sử: quá [`MAX_HISTORY`] job thì bỏ các job đã kết thúc (cũ nhất
        // trước). Job đang chạy không bao giờ bị bỏ.
        if all.len() >= MAX_HISTORY {
            let overflow = all.len() + 1 - MAX_HISTORY;
            let mut finished: Vec<String> = Vec::new();
            for job in all.iter() {
                let done = match job.state.try_lock() {
                    Ok(state) => !matches!(*state, JobState::Running),
                    Err(_) => false,
                };
                if done {
                    finished.push(job.id.clone());
                    if finished.len() >= overflow {
                        break;
                    }
                }
            }
            all.retain(|job| !finished.contains(&job.id));
        }
        let running = all
            .iter()
            .filter(|job| {
                matches!(
                    job.state.try_lock(),
                    Ok(state) if matches!(*state, JobState::Running)
                )
            })
            .count();
        if running >= MAX_RUNNING {
            return Err(ToolError::Io(format!(
                "đã có {running} job nền đang chạy (trần {MAX_RUNNING}) — xem/kết thúc bớt bằng `shell_bg_output`/`shell_bg_kill` trước khi tạo job mới"
            )));
        }

        let (mut cmd, container): (tokio::process::Command, Option<String>) = match sandbox.mode() {
            SandboxMode::Docker => {
                let name = Sandbox::next_container_name();
                let args = sandbox.docker_run_args(&name, command);
                let mut cmd = tokio::process::Command::new("docker");
                cmd.args(&args);
                (cmd, Some(name))
            }
            SandboxMode::Host => {
                let (program, args) = host_shell_args(command);
                let mut cmd = tokio::process::Command::new(program);
                cmd.args(&args).current_dir(sandbox.workspace_path());
                (cmd, None)
            }
        };
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| ToolError::Io(format!("không khởi động được lệnh nền: {e}")))?;
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let n = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let job = Arc::new(BgJob {
            id: format!("bg-{n}"),
            started_at: Instant::now(),
            stdout: Arc::new(Mutex::new(StreamBuf::default())),
            stderr: Arc::new(Mutex::new(StreamBuf::default())),
            state: Mutex::new(JobState::Running),
            child: Mutex::new(Some(child)),
            container,
        });
        if let Some(pipe) = stdout_pipe {
            tokio::spawn(read_stream(pipe, Arc::clone(&job.stdout)));
        }
        if let Some(pipe) = stderr_pipe {
            tokio::spawn(read_stream(pipe, Arc::clone(&job.stderr)));
        }
        tokio::spawn(finish_watch(Arc::clone(&job)));
        all.push(Arc::clone(&job));
        Ok(job)
    }

    /// Tra job theo id; kèm thực thi lười trần tuổi [`MAX_AGE_SECS`].
    async fn get(&self, id: &str) -> Result<Arc<BgJob>, ToolError> {
        let all = self.jobs.lock().await;
        let job = all
            .iter()
            .find(|job| job.id == id)
            .cloned()
            .ok_or_else(|| {
                let live: Vec<&str> = all.iter().map(|job| job.id.as_str()).collect();
                ToolError::InvalidArgs(format!(
                    "không có job nền `{id}` — job còn trong bảng: {}",
                    if live.is_empty() {
                        "(không có)".to_string()
                    } else {
                        live.join(", ")
                    }
                ))
            })?;
        drop(all);
        let expired = {
            let state = job.state.lock().await;
            matches!(*state, JobState::Running) && job.started_at.elapsed().as_secs() > MAX_AGE_SECS
        };
        if expired {
            stop_job(
                &job,
                JobState::Failed(format!(
                    "vượt tuổi thọ tối đa {MAX_AGE_SECS}s — bị dừng tự động"
                )),
            )
            .await;
        }
        Ok(job)
    }
}

/// Đọc một pipe vào bộ đệm cho tới khi stream đóng (job xong) hoặc lỗi I/O.
async fn read_stream<R: tokio::io::AsyncRead + Unpin>(mut pipe: R, buf: Arc<Mutex<StreamBuf>>) {
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.lock().await.push(&chunk[..n]),
        }
    }
}

/// Chờ job kết thúc và ghi trạng thái — giữ nguyên nếu job đã bị kill trước đó.
async fn finish_watch(job: Arc<BgJob>) {
    let child = job.child.lock().await.take();
    let Some(mut child) = child else {
        return;
    };
    let outcome = child.wait().await;
    let mut state = job.state.lock().await;
    if matches!(*state, JobState::Running) {
        *state = match outcome {
            Ok(status) => JobState::Exited(status.code()),
            Err(e) => JobState::Failed(e.to_string()),
        };
    }
}

/// Kill job: kill tiến trình (và container nếu docker mode), cập nhật trạng thái.
/// Job đã kết thúc trước đó thì giữ nguyên trạng thái cũ — không ghi đè exit code.
async fn stop_job(job: &BgJob, final_state: JobState) {
    let child = job.child.lock().await.take();
    if let Some(mut child) = child {
        let _ = child.kill().await; // kill() đã wait() — không để lại zombie
    }
    if let Some(name) = &job.container {
        kill_container(name).await;
    }
    let mut state = job.state.lock().await;
    if matches!(*state, JobState::Running) {
        *state = final_state;
    }
}

// ---------------------------------------------------------------------------
// Format output
// ---------------------------------------------------------------------------

/// Giữ **đuôi** `s` tối đa `max_chars` ký tự (ranh giới UTF-8 an toàn).
fn tail_chars(s: &str, max_chars: usize) -> (&str, usize) {
    let total = s.chars().count();
    if total <= max_chars {
        return (s, 0);
    }
    let skip = total - max_chars;
    let idx = s
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or_else(|| s.len());
    (&s[idx..], skip)
}

/// Format một stream đã khoá: ghi chú phần bị bỏ đầu + cắt giữ đuôi.
fn format_stream(label: &str, buf: &StreamBuf) -> String {
    if buf.bytes.is_empty() {
        return String::new();
    }
    let head_note = if buf.dropped_head > 0 {
        format!(
            "[đã bỏ {} byte đầu do vượt trần bộ đệm]\n",
            buf.dropped_head
        )
    } else {
        String::new()
    };
    let text = String::from_utf8_lossy(&buf.bytes).into_owned();
    let (kept, cut) = tail_chars(&text, OUTPUT_MAX_CHARS);
    let cut_note = if cut > 0 {
        format!("\n[đã bỏ {cut} ký tự đầu, giữ {OUTPUT_MAX_CHARS} ký tự gần nhất]")
    } else {
        String::new()
    };
    format!("{head_note}--- {label} ---\n{kept}{cut_note}")
}

fn format_state(job: &BgJob, state: &JobState) -> String {
    let elapsed = job.started_at.elapsed().as_secs();
    match state {
        JobState::Running => format!("đang chạy ({elapsed}s)"),
        JobState::Exited(Some(code)) => format!("đã kết thúc, exit code: {code}"),
        JobState::Exited(None) => "đã kết thúc, bị kill bởi signal".to_string(),
        JobState::Killed => "đã bị huỷ (shell_bg_kill hoặc dừng tự động)".to_string(),
        JobState::Failed(e) => format!("lỗi: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Tham số các tool
// ---------------------------------------------------------------------------

/// Tham số của `run_shell_bg`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunShellBgParams {
    /// Câu lệnh shell chạy nền trong sandbox (cùng ngữ nghĩa `run_shell`). Dùng
    /// cho lệnh dài: build, test suite, server tạm. Tool trả về job id NGAY, không
    /// chờ lệnh xong — đọc output/tình trạng bằng `shell_bg_output`.
    pub command: String,
}

/// Tham số của `shell_bg_output`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellBgOutputParams {
    /// Id job nhận được từ `run_shell_bg` (ví dụ `bg-1`).
    pub job_id: String,
}

/// Tham số của `shell_bg_kill`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellBgKillParams {
    /// Id job cần huỷ (ví dụ `bg-1`).
    pub job_id: String,
}

// ---------------------------------------------------------------------------
// Dựng ba tool
// ---------------------------------------------------------------------------

/// Dựng bộ ba tool shell nền gắn với một bảng job chung.
///
/// Tham số giống [`crate::shell::run_shell_for_projects`] (sandbox mặc định +
/// sandbox theo project + watch cell mode) — cùng sandbox, cùng chính sách risk.
#[must_use]
pub fn shell_bg_tools(
    sandbox: Arc<Sandbox>,
    by_project: BTreeMap<String, Arc<Sandbox>>,
    mode: watch::Receiver<SandboxMode>,
) -> Vec<Arc<dyn Tool>> {
    let jobs = Arc::new(BgJobs::default());
    vec![
        run_shell_bg_tool(Arc::clone(&jobs), sandbox, by_project, mode),
        shell_bg_output_tool(Arc::clone(&jobs)),
        shell_bg_kill_tool(jobs),
    ]
}

/// Risk của `run_shell_bg` — **y hệt** `run_shell` (shell.rs): docker Safe khi
/// không có mạng, host theo whitelist Q8; tính lại mỗi call từ watch cell.
fn bg_risk(
    risk_mode: &watch::Receiver<SandboxMode>,
    risk_sandbox: &Arc<Sandbox>,
    args: &serde_json::Value,
) -> Risk {
    match *risk_mode.borrow() {
        SandboxMode::Docker => {
            Sandbox::base_risk_for(SandboxMode::Docker, risk_sandbox.network_enabled())
        }
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
    }
}

fn run_shell_bg_tool(
    jobs: Arc<BgJobs>,
    sandbox: Arc<Sandbox>,
    by_project: BTreeMap<String, Arc<Sandbox>>,
    mode: watch::Receiver<SandboxMode>,
) -> Arc<dyn Tool> {
    let risk_mode = mode.clone();
    let risk_sandbox = Arc::clone(&sandbox);
    Arc::new(
        TypedTool::with_risk_fn(
            "run_shell_bg",
            move |args: &serde_json::Value| bg_risk(&risk_mode, &risk_sandbox, args),
            move |ctx: &bean_tools::ToolCtx, p: RunShellBgParams| {
                let mode = mode.clone();
                let jobs = jobs.clone();
                let sandbox = by_project
                    .get(&ctx.project)
                    .cloned()
                    .unwrap_or_else(|| sandbox.clone());
                async move {
                    if p.command.trim().is_empty() {
                        return Err(ToolError::InvalidArgs(
                            "command rỗng — cần một câu lệnh để chạy nền".to_string(),
                        ));
                    }
                    let sandbox = sandbox.with_mode(*mode.borrow());
                    let job = jobs.start(&sandbox, &p.command).await?;
                    Ok(format!(
                        "đã khởi động job nền `{}` cho lệnh: {}\nđọc tình trạng/output bằng tool `shell_bg_output` (job_id = \"{}\"); huỷ bằng `shell_bg_kill`. Job không bền qua restart Bean.",
                        job.id, p.command, job.id
                    ))
                }
            },
        )
        .requires_tags(["dev-write", "infra-scan"]),
    )
}

fn shell_bg_output_tool(jobs: Arc<BgJobs>) -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "shell_bg_output",
            Risk::Safe,
            move |_ctx: &bean_tools::ToolCtx, p: ShellBgOutputParams| {
                let jobs = jobs.clone();
                async move {
                    let job = jobs.get(&p.job_id).await?;
                    let state = job.state.lock().await;
                    let head = format_state(&job, &state);
                    drop(state);
                    let mut out = format!("job {}: {head}", job.id);
                    let stdout_guard = job.stdout.lock().await;
                    let stdout = format_stream("stdout", &stdout_guard);
                    drop(stdout_guard);
                    let stderr_guard = job.stderr.lock().await;
                    let stderr = format_stream("stderr", &stderr_guard);
                    drop(stderr_guard);
                    if !stdout.is_empty() {
                        out.push('\n');
                        out.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        out.push('\n');
                        out.push_str(&stderr);
                    }
                    if stdout.is_empty() && stderr.is_empty() {
                        out.push_str("\n(không có output)");
                    }
                    Ok(out)
                }
            },
        )
        // Output của lệnh là dữ liệu ngoài lõi (mục 15.4) — bọc thẻ untrusted.
        .untrusted()
        .requires_tags(["dev-write", "infra-scan"]),
    )
}

fn shell_bg_kill_tool(jobs: Arc<BgJobs>) -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "shell_bg_kill",
            Risk::Safe,
            move |_ctx: &bean_tools::ToolCtx, p: ShellBgKillParams| {
                let jobs = jobs.clone();
                async move {
                    let job = jobs.get(&p.job_id).await?;
                    let already = job.child.lock().await.is_none();
                    if already {
                        let state = job.state.lock().await;
                        return Ok(match &*state {
                            JobState::Exited(Some(code)) => format!(
                                "job `{}` đã kết thúc trước đó (exit code: {code}) — không có gì để huỷ",
                                job.id
                            ),
                            _ => format!(
                                "job `{}` đã kết thúc trước đó — không có gì để huỷ",
                                job.id
                            ),
                        });
                    }
                    stop_job(&job, JobState::Killed).await;
                    Ok(format!(
                        "đã huỷ job `{}` (kill tiến trình{}). Output đã gom nằm trong `shell_bg_output`.",
                        job.id,
                        if job.container.is_some() {
                            " + docker kill container"
                        } else {
                            ""
                        }
                    ))
                }
            },
        )
        .requires_tags(["dev-write", "infra-scan"]),
    )
}

// ---------------------------------------------------------------------------
// Test — host mode thật (không cần docker): spawn, đọc output, kill, whitelist
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::paths::CapWorkspace;
    use bean_tools::ToolCtx;
    use bean_types::config::SandboxConfig;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn host_sandbox(ws: &tempfile::TempDir) -> Arc<Sandbox> {
        let cfg = SandboxConfig {
            mode: SandboxMode::Host,
            timeout_seconds: 30,
            ..SandboxConfig::default()
        };
        Arc::new(Sandbox::new(cfg, ws.path().to_path_buf()))
    }

    fn ctx(ws: Arc<CapWorkspace>) -> ToolCtx {
        ToolCtx::for_project(
            ws,
            bean_types::SessionId::new(1),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[tokio::test]
    async fn bg_job_runs_and_streams_output() {
        let ws = tempfile::tempdir().unwrap();
        let cap = Arc::new(CapWorkspace::open(ws.path().to_path_buf()).unwrap());
        let tools = shell_bg_tools(
            host_sandbox(&ws),
            BTreeMap::new(),
            watch::channel(SandboxMode::Host).1,
        );
        let (start, output) = (tools[0].clone(), tools[1].clone());

        let reply = start
            .call(
                &ctx(Arc::clone(&cap)),
                serde_json::json!({"command": "printf xin; printf chaoban 1>&2"}),
            )
            .await
            .unwrap();
        assert!(reply.contains("bg-1"), "reply: {reply}");

        // chờ job xong (tối đa 5s)
        let mut final_out = String::new();
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            final_out = output
                .call(
                    &ctx(Arc::clone(&cap)),
                    serde_json::json!({"job_id": "bg-1"}),
                )
                .await
                .unwrap();
            if final_out.contains("exit code: 0") {
                break;
            }
        }
        assert!(final_out.contains("exit code: 0"), "out: {final_out}");
        assert!(final_out.contains("xin"), "stdout thiếu: {final_out}");
        assert!(final_out.contains("chaoban"), "stderr thiếu: {final_out}");
    }

    #[tokio::test]
    async fn bg_kill_stops_running_job() {
        let ws = tempfile::tempdir().unwrap();
        let cap = Arc::new(CapWorkspace::open(ws.path().to_path_buf()).unwrap());
        let tools = shell_bg_tools(
            host_sandbox(&ws),
            BTreeMap::new(),
            watch::channel(SandboxMode::Host).1,
        );
        let (start, output, kill) = (tools[0].clone(), tools[1].clone(), tools[2].clone());

        start
            .call(
                &ctx(Arc::clone(&cap)),
                serde_json::json!({"command": "sleep 30"}),
            )
            .await
            .unwrap();
        let reply = kill
            .call(
                &ctx(Arc::clone(&cap)),
                serde_json::json!({"job_id": "bg-1"}),
            )
            .await
            .unwrap();
        assert!(reply.contains("đã huỷ"), "reply: {reply}");

        let out = output
            .call(
                &ctx(Arc::clone(&cap)),
                serde_json::json!({"job_id": "bg-1"}),
            )
            .await
            .unwrap();
        assert!(out.contains("huỷ"), "out: {out}");
    }

    #[tokio::test]
    async fn unknown_job_id_is_invalid_args() {
        let ws = tempfile::tempdir().unwrap();
        let cap = Arc::new(CapWorkspace::open(ws.path().to_path_buf()).unwrap());
        let tools = shell_bg_tools(
            host_sandbox(&ws),
            BTreeMap::new(),
            watch::channel(SandboxMode::Host).1,
        );
        let err = tools[1]
            .call(
                &ctx(Arc::clone(&cap)),
                serde_json::json!({"job_id": "bg-99"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)), "err: {err:?}");
    }

    #[test]
    fn host_risk_follows_whitelist() {
        let ws = tempfile::tempdir().unwrap();
        let cfg = SandboxConfig {
            mode: SandboxMode::Host,
            timeout_seconds: 30,
            host_allow_programs: vec!["sleep".to_string()],
            ..SandboxConfig::default()
        };
        let sandbox = Arc::new(Sandbox::new(cfg, ws.path().to_path_buf()));
        let tools = shell_bg_tools(
            sandbox,
            BTreeMap::new(),
            watch::channel(SandboxMode::Host).1,
        );
        assert_eq!(
            tools[0].risk(&serde_json::json!({"command": "sleep 30"})),
            Risk::Safe
        );
        assert_eq!(
            tools[0].risk(&serde_json::json!({"command": "curl evil; rm -rf /"})),
            Risk::Dangerous
        );
    }
}
