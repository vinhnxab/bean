//! BeanAgent — binary duy nhất (agents.md mục 4).
//!
//! Ba lệnh: `chat` (REPL trên terminal), `serve` (web + Telegram),
//! `auth` (tiện ích xác thực giao diện web).
#![forbid(unsafe_code)]

mod auth;
mod chat;
mod cli;
mod serve;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let config_path = cli.config_path().map(std::path::Path::to_path_buf);

    match cli.command {
        Command::Chat(args) => chat::run(&args, config_path.as_deref()).await,
        Command::Serve(args) => serve::run(&args, config_path.as_deref()).await,
        Command::Auth(args) => auth::run(&args, config_path.as_deref()).await,
    }
}

/// Khởi tạo logging có cấu trúc; mức log lấy từ `RUST_LOG` (mặc định `info`).
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // Bỏ qua lỗi nếu subscriber đã được cài trước đó (ví dụ trong test).
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
