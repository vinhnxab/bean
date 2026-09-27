//! BeanAgent — binary duy nhất (agents.md mục 4).
//!
//! Ba lệnh: `chat` (REPL trên terminal), `serve` (web + Telegram),
//! `auth` (tiện ích xác thực), `mcp` (MCP server read-only cho agent khác).
#![forbid(unsafe_code)]

mod auth;
mod chat;
mod cli;
mod mcp;
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
        Command::Mcp(args) => mcp::run(args.command, config_path.as_deref()).await,
    }
}

/// Khởi tại logging có cấu trúc; mức log lấy từ `RUST_LOG` (mặc định `info`).
/// Log ghi ra **stderr**, không phải stdout. Đây không phải sở thích: ở `mcp serve`
/// (stdio) thì stdout **chính là** kênh JSON-RPC — một dòng log trộn vào đó khiến client
/// không đọc được phản hồi nào. Tương tự, stdout của `chat` là nơi hiển thị cho người
/// dùng nên log cũng không được chen vào đó.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // JSON giữ field có cấu trúc của `tracing` để systemd/journalctl thu thập ổn định.
    // Secret không được ghi bằng `%Debug`/`%?`; các error đã đi qua redaction ở lớp ghi.
    let _ = tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
