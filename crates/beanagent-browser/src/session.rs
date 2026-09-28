//! Vòng đời tiến trình Chrome (M26).
//!
//! # Vì sao MỘT tiến trình dùng chung, không phải mỗi tool call một cái
//!
//! * `Browser::launch` mất khoảng 1–3 giây. Một lượt agent gọi 5–10 tool browser thì
//!   mất 5–30 giây chỉ để khởi động.
//! * Chrome chết giữa chừng rồi bật lại sẽ **mất hết state phiên**: cookie,
//!   localStorage, trạng thái SPA. QA không test được luồng nhiều bước.
//!
//! # Bốn lớp dọn — không để sót tiến trình con
//!
//! 1. **Bình thườn** — [`SessionManager::shutdown`] gọi [`Browser::kill`], hàm này
//!    kill **và** chờ kết thúc nên không để lại zombie.
//! 2. **Idle** — quá `idle_timeout_seconds` thì tự tắt, mở lại ở lần gọi sau.
//!    Chrome headless ngốn vài trăm MB, giữ sống vô tận là rò rỉ.
//! 3. **Crash** — Bean chết giữa chừng thì không ai gọi được `kill`. Lớp này dọn
//!    tiến trình mồ côi ở lần khởi động kế tiếp: đọc `DevToolsActivePort` trong
//!    profile, lấy PID, và **chỉ kill khi `/proc/<pid>/cmdline` chứa đúng
//!    `--user-data-dir` của ta** — chặn việc PID bị tái sử dụng dẫn tới kill nhầm
//!    tiến trình khác.
//! 4. **Lỗi spawn** — `Browser::launch` tự kill con khi không kết nối được CDP.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig as ChromeConfig};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use beanagent_types::config::BrowserConfig as BrowserSettings;

/// Lỗi khi khởi động hoặc dùng Chrome.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// Không tìm thấy Chrome/Chromium đã cài sẵn.
    #[error(
        "không tìm thấy Chrome/Chromium đã cài sẵn. Cài `google-chrome-stable` hoặc \
         `chromium`, hoặc đặt [browser].executable_path trong BeanAgent.toml. \
         Bean KHÔNG tự tải Chrome lúc chạy."
    )]
    ExecutableNotFound,
    /// Đường dẫn đã khai báo nhưng không tồn tại / không phải file.
    #[error("executable_path `{0}` không tồn tại hoặc không phải file thực thi")]
    ExecutableMissing(String),
    /// Không khởi động được (sandbox, thiếu thư viện, quá nhiều tiến trình...).
    #[error("không khởi động được Chrome: {0}")]
    Launch(String),
    /// Chrome đã tắt giữa lúc dùng.
    #[error("Chrome đã đóng; hãy gọi lại tool để mở phiên mới")]
    SessionGone,
}

struct Running {
    /// `Browser` **không** `Clone` (giữ `Child` và sender nội bộ), nên chia sẻ qua
    /// `Arc<Mutex<…>>`. Đa số API của `Browser` nhận `&self` nên khoá hầu như không
    /// bị tranh chấp; `kill()` cần `&mut` nên chỉ dọn dẹp mới khoá lâu.
    browser: Arc<Mutex<Browser>>,
    /// Task bơm sự kiện CDP — `chromiumoxide` yêu cầu có ai poll handler.
    handler: tokio::task::JoinHandle<()>,
    last_used: Instant,
    /// Trang hiện tại dùng chung cho cả chuỗi lệnh (navigate → click → fill).
    ///
    /// Không có khái niệm "trang hiện tại" thì mỗi tool mở tab riêng và chuỗi lệnh
    /// QA vỡ ngay: click xong rồi fill thì fill vào tab khác. Đây là trạng thái
    /// **dùng chung tiến trình**, không phải theo phiên hội thoại — cùng một cách
    /// `SessionPolicy` dùng chung cho cả phiên.
    current_page: Option<chromiumoxide::Page>,
    /// Origin của trang hiện tại, để tool hành động **không mang URL** (click,
    /// fill, press key) tra được mức rủi ro mà không cần hỏi CDP.
    current_origin: Option<crate::origin::Origin>,
}

/// Quản lý tiến trình Chrome dùng chung cho cả tiến trình Bean.
pub struct SessionManager {
    settings: BrowserSettings,
    profile_dir: PathBuf,
    inner: Mutex<Option<Running>>,
    /// Bản cache origin đồng bộ cho [`Self::current_origin_sync`].
    origin_cache: std::sync::RwLock<Option<crate::origin::Origin>>,
    shutdown: CancellationToken,
}

impl SessionManager {
    /// Dựng manager.
    ///
    /// `profile_dir` là thư mục `--user-data-dir` **cô lập** của Bean, tuyệt đối không
    /// phải profile Chrome cá nhân của hệ thống.
    #[must_use]
    pub fn new(settings: BrowserSettings, profile_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            settings,
            profile_dir,
            inner: Mutex::new(None),
            origin_cache: std::sync::RwLock::new(None),
            shutdown: CancellationToken::new(),
        })
    }

    /// Thư mục profile đang dùng (để test assert cô lập).
    #[must_use]
    pub fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    /// Token huỷ khi Bean dừng.
    #[must_use]
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Dọn Chrome mồ côi do lần chạy trước Bean chết không kịp tắt.
    ///
    /// # Errors
    /// Không lỗi: đây là best-effort, mọi thất bại chỉ log.
    pub async fn reap_orphans(&self) {
        let port_file = self.profile_dir.join("DevToolsActivePort");
        let Ok(raw) = tokio::fs::read_to_string(&port_file).await else {
            return; // Chưa từng chạy Chrome: không có gì để dọn.
        };
        let Some(pid) = raw
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<i32>().ok())
        else {
            return;
        };
        if pid <= 1 {
            return;
        }
        match verify_and_kill(pid, &self.profile_dir).await {
            Ok(true) => tracing::warn!(pid, "đã dọn tiến trình Chrome mồ côi của lần chạy trước"),
            Ok(false) => {}
            Err(error) => tracing::debug!(pid, error = %error, "không dọn được Chrome mồ côi"),
        }
    }

    /// Lấy browser đang chạy, khởi động nếu cần.
    ///
    /// Tự tắt và mở lại nếu đã idle quá lâu.
    ///
    /// # Errors
    /// [`LaunchError`] khi không tìm thấy/khởi động được Chrome.
    pub async fn browser(&self) -> Result<Arc<Mutex<Browser>>, LaunchError> {
        let mut guard = self.inner.lock().await;

        // Idle: tắt để giải phóng bộ nhớ rồi mở lại.
        let idle = Duration::from_secs(u64::from(self.settings.idle_timeout_seconds));
        let stale = guard
            .as_ref()
            .is_some_and(|running| !idle.is_zero() && running.last_used.elapsed() > idle);
        if stale {
            tracing::info!("Chrome idle quá hạn — tắt và mở lại");
            shut_down(guard.take()).await;
        }

        if let Some(running) = guard.as_mut() {
            running.last_used = Instant::now();
            return Ok(Arc::clone(&running.browser));
        }

        let (browser, handler) = self.launch().await?;
        let browser = Arc::new(Mutex::new(browser));
        guard.replace(Running {
            browser: Arc::clone(&browser),
            handler,
            last_used: Instant::now(),
            current_page: None,
            current_origin: None,
        });
        Ok(browser)
    }

    /// Lấy trang hiện tại, mở tab mới nếu chưa có.
    ///
    /// # Errors
    /// [`LaunchError`] khi Chrome lỗi, hoặc [`LaunchError::SessionGone`] khi trang
    /// đã bị đóng từ bên ngoài.
    pub async fn page(&self) -> Result<chromiumoxide::Page, LaunchError> {
        // `browser()` tự khoá `inner` rồi thả ra trước khi trả về, nên gọi nó
        // trước rồi mới khoá lại ở đây là an toàn (không khoá lồng nhau).
        let browser = self.browser().await?;
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.as_mut()
            && let Some(page) = running.current_page.clone()
        {
            // Kiểm tra trang còn sống: người dùng (hoặc chính trang) có thể đã đóng.
            if page.url().await.is_ok() {
                running.last_used = Instant::now();
                return Ok(page);
            }
        }
        let page = browser
            .lock()
            .await
            .new_page("about:blank")
            .await
            .map_err(|e| LaunchError::Launch(format!("không mở được tab: {e}")))?;
        page.enable_runtime()
            .await
            .map_err(|e| LaunchError::Launch(format!("không bật được Runtime domain: {e}")))?;
        if let Some(running) = guard.as_mut() {
            running.current_page = Some(page.clone());
            // `about:blank` không có origin ⇒ `None`, tức fail-closed cho tới khi
            // có `browser_navigate` thật sự đặt origin.
            running.current_origin = None;
        }
        Ok(page)
    }

    /// Origin của trang hiện tại (`None` khi chưa navigate hoặc URL không phải web).
    ///
    /// Đây là đường **fail-closed**: `None` ⇒ mọi tool hành động không mang URL đều
    /// `Dangerous`.
    pub async fn current_origin(&self) -> Option<crate::origin::Origin> {
        self.inner
            .lock()
            .await
            .as_ref()
            .and_then(|running| running.current_origin.clone())
    }

    /// Origin hiện tại đọc **đồng bộ**, để [`crate::act_tools`] gọi được trong
    /// `Tool::risk` (hàm không async).
    ///
    /// Không thể lấy khoá `Mutex` async ở đây, nên dùng `std::sync::RwLock` riêng cho
    /// bản cache. Nó chỉ chứa một `Option<Origin>` (Copy-ish, rất nhỏ) nên không có
    /// rủi ro contention thực tế, và **luôn trả về bản cũ nhất** thay vì trả sai:
    /// việc lệch chỉ có thể xảy ra giữa hai lệnh, mà cả hai đều bị policy chặn lại.
    #[must_use]
    pub fn current_origin_sync(&self) -> Option<crate::origin::Origin> {
        self.origin_cache
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Ghi nhớ origin sau khi điều hướng (cập nhật cả bản cache đồng bộ).
    pub async fn set_current_origin(&self, origin: Option<crate::origin::Origin>) {
        if let Ok(mut guard) = self.origin_cache.write() {
            *guard = origin.clone();
        }
        if let Some(running) = self.inner.lock().await.as_mut() {
            running.current_origin = origin;
        }
    }

    /// Khởi động Chrome mới theo cấu hình.
    ///
    /// Trả kèm task bơm sự kiện CDP để chủ sở hữu quản lý vòng đời được.
    async fn launch(&self) -> Result<(Browser, tokio::task::JoinHandle<()>), LaunchError> {
        // Lớp dọn 3: trước khi mở, dọn tàn dư của lần chạy trước.
        self.reap_orphans().await;

        let executable = resolve_executable(&self.settings.executable_path)?;
        std::fs::create_dir_all(&self.profile_dir).map_err(|e| {
            LaunchError::Launch(format!(
                "không tạo được thư mục profile `{}`: {e}",
                self.profile_dir.display()
            ))
        })?;

        let mut builder = ChromeConfig::builder()
            .user_data_dir(&self.profile_dir)
            .launch_timeout(Duration::from_secs(self.settings.launch_timeout_seconds))
            // KHÔNG gọi `no_sandbox()` — sandbox renderer của Chromium là lớp phòng
            // thủ chính chống exploit-render-escape (mục 15.2). Tắt nó để "chạy được
            // trong container" là đánh đổi sai: mất lớp phòng thủ để đổi lấy tiện lợi.
            .args(BASE_ARGS.iter().map(|arg| (*arg).to_string()));

        // `HeadlessMode` **không** được crate export ra ngoài, nên chỉ dùng được hai
        // builder method công khai: `new_headless_mode()` = headless kiểu mới (Chrome
        // >= 112, không cần X server), `with_head()` = có cửa sổ.
        builder = if self.settings.headless {
            builder.new_headless_mode()
        } else {
            builder.with_head()
        };

        // `chrome_executable` đặt executable đã phát hiện ở **cả hai** nhánh: khi
        // người dùng khai `executable_path` thì dùng đường dẫn đó, còn lại dùng kết
        // quả của bộ phát hiện. Không để crate tự dò lần thứ (lệch thông điệp lỗi).
        let config = builder
            .chrome_executable(executable)
            .build()
            .map_err(|e| LaunchError::Launch(format!("cấu hình Chrome không hợp lệ: {e}")))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| LaunchError::Launch(e.to_string()))?;
        // Handler là **bơm sự kiện CDP**: không ai poll thì các lệnh không bao giờ
        // nhận được phản hồi. Task này sống cùng Chrome và bị `abort` khi tắt.
        let pump = tokio::spawn(async move {
            use futures_util::StreamExt;
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });
        Ok((browser, pump))
    }

    /// Tắt Chrome và giải phóng tài nguyên.
    pub async fn shutdown(&self) {
        let taken = self.inner.lock().await.take();
        shut_down(taken).await;
    }
}

/// Cờ khởi động: giữ Chrome không chạm vào hạ tầng người dùng và không tự gửi
/// telemetry, đồng thời trần tài nguyên (M26, quyết định điểm 8).
const BASE_ARGS: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-background-networking",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--renderer-process-limit=4",
    "--js-flags=--max-old-space-size=512",
    "--disable-features=Translate,OptimizationHints",
];

/// Tắt một phiên Chrome đang chạy.
///
/// `kill()` cần `&mut Browser`, mà `Browser` được chia sẻ qua `Arc` nên không thể
/// gọi khi còn `Arc` khác giữ. Vì vậy `Running` giữ `Arc<Mutex<Browser>>`: các tool
/// chỉ cần `&self` (đa số API của `Browser` vốn đã nhận `&self`), còn dọn dẹp thì
/// khoá tạm thời để lấy `&mut`.
async fn shut_down(running: Option<Running>) {
    let Some(running) = running else {
        return;
    };
    // Ưu tiên `kill`: graceful `close` có thể treo nếu trang đang chạy script dài,
    // còn đây là lúc Bean đang tắt — chấp nhận bỏ dở để không giữ tiến trình mồ côi.
    if let Ok(mut guard) = running.browser.try_lock() {
        // `kill` tự chờ tiến trình kết thúc nên không để lại zombie.
        let outcome = guard.kill().await;
        match outcome {
            Some(Ok(())) => {}
            Some(Err(error)) => tracing::warn!(error = %error, "kill Chrome thất bại"),
            // `None` = browser được `connect` (không spawn) ⇒ không có gì để dọn.
            None => {}
        }
    } else {
        tracing::warn!("còn tool đang giữ Chrome lúc tắt — bỏ qua kill chủ động");
    }
    running.handler.abort();
}

/// Xác định đường dẫn Chrome đã cài sẵn.
///
/// # Errors
/// [`LaunchError::ExecutableMissing`] nếu người dùng khai sai, hoặc
/// [`LaunchError::ExecutableNotFound`] nếu không tìm thấy ở đâu.
fn resolve_executable(configured: &str) -> Result<PathBuf, LaunchError> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        if path.is_file() {
            return Ok(path);
        }
        return Err(LaunchError::ExecutableMissing(trimmed.to_string()));
    }
    // Dùng chính bộ phát hiện của chromiumoxide: biến `CHROME` → tên binary trong
    // PATH → đường dẫn cố định. **Phát hiện binary đã cài, không phải tải.**
    chromiumoxide::detection::default_executable(chromiumoxide::detection::DetectionOptions {
        msedge: false,
        unstable: false,
    })
    .map_err(|_| LaunchError::ExecutableNotFound)
}

/// Kiểm tra PID có thật sự là Chrome của Bean rồi mới kill.
///
/// Chặn nhầm PID: `/proc/<pid>/cmdline` phải chứa **đúng** `--user-data-dir` mà ta
/// đang dùng. Không có bước này, PID bị tái sử dụng sẽ khiến ta kill nhầm một tiến
/// trình không liên quan của người dùng.
async fn verify_and_kill(pid: i32, profile_dir: &Path) -> Result<bool, String> {
    let cmdline_path = format!("/proc/{pid}/cmdline");
    let raw = match tokio::fs::read(&cmdline_path).await {
        Ok(raw) => raw,
        // Tiến trình đã chết là kết quả tốt, không phải lỗi.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("không đọc được cmdline: {error}")),
    };
    // Trong `/proc`, các argv được nối bằng byte NUL.
    let joined = String::from_utf8_lossy(&raw).replace('\0', " ");
    if !joined.contains("--user-data-dir") || !joined.contains(&profile_dir.display().to_string()) {
        tracing::debug!(pid, "PID không thuộc Chrome của Bean — bỏ qua");
        return Ok(false);
    }
    kill_pid(pid).await
}

/// Gửi tín hiệu cho PID và chờ nó thật sự biến mất.
///
/// Dùng `kill` (tiện ích hệ thống) thay vì crate `libc`: crate này `forbid(unsafe_code)`
/// và không muốn thêm dependency chỉ để gọi một syscall.
///
/// Thử `TERM` trước để Chrome tự dọn profile, rồi mới `KILL`.
///
/// # Errors
/// Trả `Ok(true)` nếu tiến trình đã biến mất, `Ok(false)` nếu vẫn còn sau cả hai tín
/// hiệu.
async fn kill_pid(pid: i32) -> Result<bool, String> {
    let proc_dir = format!("/proc/{pid}");
    for signal in ["-TERM", "-KILL"] {
        if !process_alive(&proc_dir).await {
            return Ok(true);
        }
        let _ = tokio::process::Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        for _ in 0..20 {
            if !process_alive(&proc_dir).await {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    Ok(false)
}

/// Tiến trình còn sống hay không (đọc `/proc/<pid>`).
async fn process_alive(proc_dir: &str) -> bool {
    tokio::fs::try_exists(proc_dir).await.unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use beanagent_types::config::BrowserConfig;

    fn settings() -> BrowserConfig {
        BrowserConfig {
            enabled: true,
            ..BrowserConfig::default()
        }
    }

    /// Đường dẫn profile phải là thư mục Bean quản lý, KHÔNG phải profile hệ thống.
    #[test]
    fn profile_dir_is_isolated_from_the_system_profile() {
        let manager = SessionManager::new(settings(), PathBuf::from("/data/browser/profile"));
        let profile = manager.profile_dir();
        assert_eq!(profile, Path::new("/data/browser/profile"));
        // Những đường dẫn KHÔNG được dùng — đây là dữ liệu cá nhân của người dùng.
        for forbidden in [
            ".config/google-chrome",
            ".config/chromium",
            "Library/Application Support/Google/Chrome",
        ] {
            assert!(
                !profile.to_string_lossy().contains(forbidden),
                "profile không được trỏ vào `{forbidden}`"
            );
        }
    }

    /// Không cài Chrome ở đường dẫn người dùng khai ⇒ lỗi **rõ ràng**, không phải
    /// tải Chrome về thay.
    #[tokio::test]
    async fn missing_configured_executable_is_an_error_not_a_download() {
        let manager = SessionManager::new(
            BrowserConfig {
                enabled: true,
                executable_path: "/definitely/not/a/chrome".to_string(),
                ..BrowserConfig::default()
            },
            PathBuf::from("/tmp/beanagent-browser-test-profile"),
        );
        let error = manager.browser().await.expect_err("phải báo lỗi");
        assert!(
            matches!(error, LaunchError::ExecutableMissing(_)),
            "nhận: {error}"
        );
        let text = error.to_string();
        assert!(text.contains("/definitely/not/a/chrome"), "{text}");
    }

    /// PID không thuộc Chrome của ta thì **không** được kill — đây là lớp chặn
    /// nhầm tiến trình khi PID bị tái sử dụng.
    #[tokio::test]
    async fn foreign_pid_is_never_killed() {
        // PID 1 là init, không bao giờ là Chrome của ta.
        let killed = verify_and_kill(1, Path::new("/data/browser/profile"))
            .await
            .unwrap_or(false);
        assert!(!killed, "không được kill tiến trình không phải của mình");
    }

    #[tokio::test]
    async fn reap_orphans_is_a_noop_when_no_port_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(settings(), dir.path().join("profile"));
        // Không có `DevToolsActivePort` ⇒ không có gì để dọn, không được lỗi.
        manager.reap_orphans().await;
    }
}
