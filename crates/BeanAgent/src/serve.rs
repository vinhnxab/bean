//! `BeanAgent serve` — khung "chưa cài đặt" cho tới M9/M12/M13.
//!
//! Khi hoàn thiện, lệnh này sẽ:
//! * M9: dựng `BeanAgent-web` (axum) phục vụ UI nhúng + REST + WebSocket.
//! * M12: chạy kênh Telegram (long polling) song song.
//! * M13: chạy scheduler tick 30 giây cho tác vụ định kỳ.

use std::path::Path;

use anyhow::{Result, bail};
use beanagent_types::config::Config;

use crate::cli::ServeArgs;

/// Chạy server (chưa cài đặt ở M1).
///
/// # Errors
/// Luôn trả lỗi ở M1 (kèm thông tin cấu hình đã đọc được để người dùng kiểm tra).
pub async fn run(args: &ServeArgs, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path)?;

    tracing::info!(
        provider = config.llm.provider.as_str(),
        model = %config.llm.model,
        web_enabled = config.web.enabled,
        bind = %config.web.bind,
        telegram_enabled = config.telegram.enabled,
        sandbox = config.security.sandbox.mode.as_str(),
        "cấu hình đã nạp"
    );
    if let Some(path) = args.fake_llm.as_deref() {
        tracing::info!(script = %path.display(), "--fake-llm sẽ được hỗ trợ đầy đủ ở M9");
    }

    bail!("`serve` chưa được cài đặt (M9: web server, M12: Telegram, M13: scheduler)")
}
