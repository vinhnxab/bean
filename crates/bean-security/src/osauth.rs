//! Xác minh mật khẩu tài khoản OS của tiến trình Bean (D22.2 — toggle host mode
//! từ UI step-up bằng mật khẩu máy).
//!
//! # Vì sao module này tồn tại
//!
//! Bật `security.sandbox.mode = "host"` là bỏ sandbox thật sự — agent sẽ chạy
//! lệnh với toàn bộ quyền của người dùng. Ngoài step-up mật khẩu Bean (D19.5),
//! người chủ dự án yêu cầu lớp xác nhận thứ hai gắn với **máy**: mật khẩu tài
//! khoản OS của tiến trình Bean. Máy không đặt mật khẩu ⇒ bỏ qua step-up.
//!
//! # Ranh giới bảo mật
//!
//! * Mật khẩu **không bao giờ** vào log/audit/argv ở Linux (đi qua stdin của `su`).
//!   Trên macOS `dscl` chỉ nhận mật khẩu qua argv — lộ kinh-phí trong `ps` cục bộ
//!   trong khoảnh khắc gọi, chấp nhận được vì xác minh chỉ chạy trên máy của
//!   người dùng tự yêu cầu (ghi chú ở [`real_os_auth::verify_unix_macos`]).
//! * Sai mật khẩu bị [`crate::ratelimit::RateLimiter`] khoá ở tầng endpoint —
//!   module này không tự đếm (đơn trách nhiệm).
//! * Mọi hàm **blocking** (spawn tiến trình) — caller bắt buộc bọc
//!   `tokio::task::spawn_blocking` (mục 22.8).
//!
//! # Kiến trúc (DIP)
//!
//! [`OsAuth`] là trait để endpoint web cắm giả trong test (máy CI có thể không có
//! mật khẩu tài khoản); [`RealOsAuth`] là implementation thật theo nền tảng:
//! Linux (`su` + `passwd -S`), macOS (`dscl`), Windows (FFI
//! `bean_osauth_win::verify_logon` — crate riêng vì workspace `forbid(unsafe_code)`).

use std::process::{Command, Stdio};

#[cfg(not(any(target_os = "macos", windows)))]
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

/// Trần chờ mỗi lệnh xác minh — PAM treo không được phép treo request HTTP.
#[cfg(not(any(target_os = "macos", windows)))]
const VERIFY_TIMEOUT: Duration = Duration::from_secs(15);

/// Tài khoản OS đang chạy tiến trình Bean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsAccount {
    /// Tên đăng nhập (`id -un` / `whoami` / biến môi trường).
    pub username: String,
    /// `true` = tài khoản có mật khẩu — toggle host mode phải step-up.
    /// `false` = không có mật khẩu (máy cá nhân không khóa màn hình) — bỏ qua.
    pub has_password: bool,
}

/// Lỗi xác minh mật khẩu OS. Không bao giờ chứa mật khẩu.
#[derive(Debug, Clone, Error)]
pub enum OsAuthError {
    /// Không xác định được tài khoản đang chạy tiến trình.
    #[error("không xác định được tài khoản OS của tiến trình Bean")]
    UnknownUser,
    /// Mật khẩu sai.
    #[error("mật khẩu máy không đúng")]
    InvalidPassword,
    /// Lệnh xác minh thất bại bất thường — không phân biệt được sai/đúng.
    #[error("lệnh xác minh mật khẩu OS lỗi: {0}")]
    CommandFailed(String),
    /// Lỗi I/O cấp thấp (stdin/stdout của tiến trình con).
    #[error("I/O khi xác minh mật khẩu OS: {0}")]
    Io(String),
}

/// Trạng thái mật khẩu tài khoản từ `passwd -S` (Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStatus {
    /// `P` — có mật khẩu.
    Set,
    /// `NP` — không có mật khẩu.
    None,
    /// `L` — tài khoản bị khoá.
    Locked,
}

/// Parse dòng output `passwd -S <user>`: `<user> <P|NP|L> <date> ...`.
/// Dòng khác dạng này (header, cảnh báo) ⇒ [`OsAuthError::CommandFailed`].
pub fn parse_passwd_status(output: &str) -> Result<PasswordStatus, OsAuthError> {
    let line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| OsAuthError::CommandFailed("passwd -S trả output rỗng".into()))?;
    let mut fields = line.split_whitespace();
    let _user = fields
        .next()
        .ok_or_else(|| OsAuthError::CommandFailed(format!("passwd -S lạ: {line:?}")))?;
    match fields.next() {
        Some("P") => Ok(PasswordStatus::Set),
        Some("NP") => Ok(PasswordStatus::None),
        Some("L") => Ok(PasswordStatus::Locked),
        other => Err(OsAuthError::CommandFailed(format!(
            "passwd -S trạng thái lạ: {other:?}"
        ))),
    }
}

/// Điểm cắm cho endpoint web (DIP): test cắm giả, `bean serve` cắm [`RealOsAuth`].
pub trait OsAuth: Send + Sync {
    /// Tài khoản OS của tiến trình + trạng thái mật khẩu. Blocking.
    ///
    /// # Errors
    /// [`OsAuthError`] khi không xác định được tài khoản hoặc lệnh kiểm tra lỗi.
    fn inspect(&self) -> Result<OsAccount, OsAuthError>;

    /// Xác minh mật khẩu của `username`. Blocking — caller bọc `spawn_blocking`.
    ///
    /// # Errors
    /// [`OsAuthError::InvalidPassword`] khi sai; các lỗi còn lại nghĩa là
    /// **không phân biệt được** sai/đúng — endpoint phải trả 5xx, không khoá ai.
    fn verify(&self, username: &str, password: &SecretString) -> Result<(), OsAuthError>;
}

/// Implementation thật theo nền tảng — không trạng thái, dựng mỗi lần cần.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealOsAuth;

impl OsAuth for RealOsAuth {
    fn inspect(&self) -> Result<OsAccount, OsAuthError> {
        let username = current_username()?;
        let has_password = account_has_password(&username)?;
        Ok(OsAccount {
            username,
            has_password,
        })
    }

    fn verify(&self, username: &str, password: &SecretString) -> Result<(), OsAuthError> {
        verify_platform(username, password)
    }
}

/// Tên tài khoản đang chạy tiến trình. Ưu tiên biến môi trường chuẩn của nền
/// tảng (nhanh, không spawn), fallback lệnh hệ thống.
fn current_username() -> Result<String, OsAuthError> {
    #[cfg(windows)]
    let env_name = "USERNAME";
    #[cfg(not(windows))]
    let env_name = "USER";
    if let Ok(user) = std::env::var(env_name)
        && !user.trim().is_empty()
    {
        return Ok(user.trim().to_string());
    }
    #[cfg(windows)]
    let probe = "whoami";
    #[cfg(not(windows))]
    let probe = "id -un";
    let output = Command::new(probe)
        .output()
        .map_err(|error| OsAuthError::CommandFailed(format!("{probe}: {error}")))?;
    let user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if user.is_empty() {
        return Err(OsAuthError::UnknownUser);
    }
    // `whoami` trên Windows trả `MACHINE\user` — lấy phần sau dấu `\`.
    Ok(user.rsplit('\\').next().unwrap_or(&user).to_string())
}

/// Tài khoản có mật khẩu không? Fail-closed: không trả lời được ⇒ `true`.
fn account_has_password(username: &str) -> Result<bool, OsAuthError> {
    #[cfg(target_os = "macos")]
    {
        account_has_password_macos(username)
    }
    #[cfg(windows)]
    {
        account_has_password_windows(username)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = username;
        account_has_password_linux()
    }
}

/// Linux: `passwd -S <user>` — field thứ hai là `P`/`NP`/`L`.
/// Chạy được không cần root cho **tài khoản của chính tiến trình** (helper setgid
/// `unix_chkpwd` của PAM). Không parse được ⇒ lỗi — người dùng cần biết để kiểm tra.
#[cfg(not(any(target_os = "macos", windows)))]
fn account_has_password_linux() -> Result<bool, OsAuthError> {
    let username = current_username()?;
    let output = Command::new("passwd")
        .arg("-S")
        .arg(&username)
        .output()
        .map_err(|error| OsAuthError::CommandFailed(format!("passwd: {error}")))?;
    if !output.status.success() {
        return Err(OsAuthError::CommandFailed(format!(
            "passwd -S thoát mã {:?}",
            output.status.code()
        )));
    }
    match parse_passwd_status(&String::from_utf8_lossy(&output.stdout))? {
        PasswordStatus::Set => Ok(true),
        PasswordStatus::None => Ok(false),
        PasswordStatus::Locked => Ok(true),
    }
}

/// macOS: thử xác minh với mật khẩu **rỗng** — thành công nghĩa là tài khoản
/// không có mật khẩu. `dscl` trả khác 0 cho mọi auth thất bại (không có mã riêng
/// cho "sai mật khẩu" đáng tin) — cứ coi khác 0 là có mật khẩu (fail-closed).
#[cfg(target_os = "macos")]
fn account_has_password_macos(username: &str) -> Result<bool, OsAuthError> {
    let output = Command::new("dscl")
        .args([".", "-authonly", username, ""])
        .output()
        .map_err(|error| OsAuthError::CommandFailed(format!("dscl: {error}")))?;
    Ok(!output.status.success())
}

/// Windows: thử logon bằng mật khẩu rỗng (crate FFI riêng — workspace
/// `forbid(unsafe_code)`). Thất bại ⇒ có mật khẩu (fail-closed).
#[cfg(windows)]
fn account_has_password_windows(username: &str) -> Result<bool, OsAuthError> {
    Ok(bean_osauth_win::verify_logon(username, "").is_err())
}

/// Xác minh theo nền tảng. Sai mật khẩu và lỗi hạ tầng phải **tách bạch**: chỉ
/// [`OsAuthError::InvalidPassword`] mới được phép tính vào bộ đếm khoá.
fn verify_platform(username: &str, password: &SecretString) -> Result<(), OsAuthError> {
    #[cfg(target_os = "macos")]
    {
        verify_unix_macos(username, password)
    }
    #[cfg(windows)]
    {
        verify_windows(username, password)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        verify_unix_linux(username, password)
    }
}

/// Linux: `su <user> -c true`, mật khẩu qua **stdin** (không argv, không env).
/// `su` của util-linux đọc mật khẩu từ stdin khi stdin không phải TTY. `-c true`
/// luôn thành công sau khi auth — nên **mọi** exit khác 0 là auth thất bại
/// (không khớp thông báo lỗi theo locale).
#[cfg(not(any(target_os = "macos", windows)))]
fn verify_unix_linux(username: &str, password: &SecretString) -> Result<(), OsAuthError> {
    use std::io::Write;
    let mut child = Command::new("su")
        .arg(username)
        .arg("-c")
        .arg("true")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| OsAuthError::CommandFailed(format!("su: {error}")))?;
    // Viết mật khẩu rồi đóng stdin ngay — `su` đọc tới EOF mới auth.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(password.expose_secret().as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .map_err(|error| OsAuthError::Io(error.to_string()))?;
        drop(stdin);
    }
    let output = wait_with_timeout(&mut child)?;
    if output.status.success() {
        return Ok(());
    }
    Err(OsAuthError::InvalidPassword)
}

/// macOS: `dscl . -authonly <user> <pass>`. `dscl` không nhận mật khẩu qua
/// stdin/flag riêng — argv là đường duy nhất; lộ trong `ps` cục bộ trong khoảnh
/// khắc gọi, chấp nhận trên máy cá nhân (đã ghi chú ở module doc).
#[cfg(target_os = "macos")]
fn verify_unix_macos(username: &str, password: &SecretString) -> Result<(), OsAuthError> {
    let output = Command::new("dscl")
        .args([".", "-authonly", username, password.expose_secret()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| OsAuthError::CommandFailed(format!("dscl: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(OsAuthError::InvalidPassword)
}

/// Windows: FFI `LogonUserW` (LOGON32_LOGON_INTERACTIVE — logon của chính tiến
/// trình, không cần quyền đặc biệt, không bị `LimitBlankPasswordUse` chặn khi
/// người dùng tự chạy trên máy của mình).
#[cfg(windows)]
fn verify_windows(username: &str, password: &SecretString) -> Result<(), OsAuthError> {
    match bean_osauth_win::verify_logon(username, password.expose_secret()) {
        Ok(()) => Ok(()),
        Err(bean_osauth_win::LogonError::InvalidCredentials) => Err(OsAuthError::InvalidPassword),
        Err(bean_osauth_win::LogonError::Failed(message)) => {
            Err(OsAuthError::CommandFailed(message))
        }
    }
}

/// Chờ tiến trình con với trần [`VERIFY_TIMEOUT`] — quá trần kill rồi trả lỗi
/// hạ tầng (không phải sai mật khẩu).
#[cfg(not(any(target_os = "macos", windows)))]
fn wait_with_timeout(child: &mut std::process::Child) -> Result<std::process::Output, OsAuthError> {
    let deadline = std::time::Instant::now() + VERIFY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Đã kết thúc — đọc nốt stdout/stderr (đã EOF) không chặn.
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    use std::io::Read;
                    let _ = out.read_to_end(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    use std::io::Read;
                    let _ = err.read_to_end(&mut stderr);
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(OsAuthError::CommandFailed(format!(
                        "xác minh quá {} giây — đã kill",
                        VERIFY_TIMEOUT.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(OsAuthError::Io(error.to_string())),
        }
    }
}

/// Điểm cắm giả cho test endpoint web (DIP, theo mẫu `NoopRebind`): trạng thái
/// tài khoản và mật khẩu "đúng" đặt được trực tiếp — hermetic, không đụng OS
/// thật của máy chạy CI.
#[derive(Debug, Clone)]
pub struct FakeOsAuth {
    /// Kết quả [`OsAuth::inspect`] — `Err` mô phỏng hạ tầng xác minh lỗi.
    pub account: Result<OsAccount, OsAuthError>,
    /// Mật khẩu duy nhất [`OsAuth::verify`] chấp nhận; `None` = mọi mật khẩu sai.
    pub password: Option<String>,
}

impl Default for FakeOsAuth {
    fn default() -> Self {
        Self {
            account: Ok(OsAccount {
                username: "bean".into(),
                has_password: true,
            }),
            password: Some("right".into()),
        }
    }
}

impl OsAuth for FakeOsAuth {
    fn inspect(&self) -> Result<OsAccount, OsAuthError> {
        self.account.clone()
    }

    fn verify(&self, _username: &str, password: &SecretString) -> Result<(), OsAuthError> {
        if self.password.as_deref() == Some(password.expose_secret()) {
            Ok(())
        } else {
            Err(OsAuthError::InvalidPassword)
        }
    }
}
