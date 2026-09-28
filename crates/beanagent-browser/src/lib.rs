//! # BeanAgent-browser
//!
//! Tool browser **nội bộ**: nói thẳng Chrome DevTools Protocol qua crate Rust
//! `chromiumoxide`. Không có Node, không có npm, không có MCP server ngoài.
//!
//! Ranh giới (M26):
//!
//! * **Nhóm đọc** (`browser-read`) — screenshot, console log, network, performance trace.
//!   Tag `dev-read` **hoặc** `test-run` ⇒ Developer lẫn QA đều dùng được.
//! * **Nhóm hành động** (`browser-act`) — navigate, click, fill, press key, evaluate script.
//!   Chỉ tag `test-run` ⇒ **chỉ QA**; Developer bị chặn (nguyên tắc four-eyes, y hệt
//!   `run_shell`/`write_file`).
//!
//! Ba bảo đảm then chốt, mỗi cái có test riêng:
//!
//! 1. [`origin`] — so khớp origin có wildcard hậu tố, khớp theo **ranh giới label**
//!    nên `*.dev.internal` không bao giờ khớp `dev.internal.attacker.com`.
//! 2. [`guard`] — lớp kiểm tra URL **riêng của domain browser**, không sửa
//!    `beanagent_security::ssrf` dùng chung. Ngoại lệ loopback chỉ mở khi origin
//!    nằm trong `allowed_origins`.
//! 3. [`session`] — vòng đời tiến trình Chrome dùng chung, dọn 4 lớp để không bỏ
//!    quên tiến trình con.
//!
//! Cấu hình: `[browser]` trong `BeanAgent.toml` ([`beanagent_types::config::BrowserConfig`]).
#![forbid(unsafe_code)]

pub mod act_tools;
pub mod guard;
pub mod origin;
pub mod read_tools;
pub mod session;

use std::path::PathBuf;
use std::sync::Arc;

use beanagent_types::config::BrowserConfig as BrowserSettings;

pub use act_tools::{
    ACT_TAGS, ClickTool, EvaluateScriptTool, FillTool, NavigateTool, PressKeyTool,
};
pub use guard::{BrowserGuard, GuardError, MAX_URL_CHARS};
pub use origin::{Origin, OriginPattern, OriginVerdict, OriginWhitelist, parse_origin};
pub use read_tools::{ConsoleLogsTool, NetworkTool, PerformanceTool, READ_TAGS, ScreenshotTool};
pub use session::{LaunchError, SessionManager};

/// Dựng toàn bộ bộ tool browser từ cấu hình + thư mục `data.dir`.
///
/// `data_dir` dùng để tính `--user-data-dir` **cô lập**: `data_dir/browser/profile`.
/// Tuyệt đối không dùng `~/.config/google-chrome` — đó là dữ liệu cá nhân của người
/// dùng, và Chrome sẽ ghi cookie/cache của mọi trang đã mở vào đó.
///
/// # Panics
/// Không panic: mọi lỗi I/O được trả về qua [`LaunchError`] lúc chạy tool.
#[must_use]
pub fn build_tools(
    settings: &BrowserSettings,
    data_dir: &std::path::Path,
) -> (Vec<Arc<dyn beanagent_tools::Tool>>, Arc<SessionManager>) {
    let profile_dir: PathBuf = data_dir.join("browser").join("profile");
    let session = SessionManager::new(settings.clone(), profile_dir);
    let whitelist = Arc::new(OriginWhitelist::parse(&settings.allowed_origins));
    if whitelist.is_empty() {
        tracing::warn!(
            "[browser].allowed_origins rỗng — MỌI tool hành động ngoài loopback sẽ là \
             Dangerous và địa chỉ mạng nội bộ sẽ bị chặn cứng"
        );
    }
    let guard = Arc::new(BrowserGuard::new(Arc::clone(&whitelist)));

    let tools: Vec<Arc<dyn beanagent_tools::Tool>> = vec![
        // Nhóm đọc: Developer lẫn QA.
        Arc::new(ScreenshotTool::new(
            Arc::clone(&session),
            settings.max_image_bytes,
        )),
        Arc::new(ConsoleLogsTool::new(Arc::clone(&session))),
        Arc::new(NetworkTool::new(Arc::clone(&session))),
        Arc::new(PerformanceTool::new(Arc::clone(&session))),
        // Nhóm hành động: CHỈ QA.
        Arc::new(NavigateTool::new(Arc::clone(&session), Arc::clone(&guard))),
        Arc::new(ClickTool::new(Arc::clone(&session), Arc::clone(&whitelist))),
        Arc::new(FillTool::new(Arc::clone(&session), Arc::clone(&whitelist))),
        Arc::new(PressKeyTool::new(
            Arc::clone(&session),
            Arc::clone(&whitelist),
        )),
        Arc::new(EvaluateScriptTool::new(Arc::clone(&session))),
    ];
    (tools, session)
}
