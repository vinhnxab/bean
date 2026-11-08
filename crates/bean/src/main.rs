//! Bean — binary duy nhất (agents.md mục 4).
//!
//! Các lệnh: `chat` (REPL trên terminal), `serve` (web + Telegram), `auth` (tiện ích
//! xác thực), `mcp` (MCP server read-only cho agent khác), `secret` + `config`
//! (M29: kho secret thống nhất và migrate key), `doctor` (M29: kiểm tra sức khoẻ
//! cấu hình).
#![forbid(unsafe_code)]

mod auth;
mod chat;
mod cli;
mod config_cmd;
mod doctor;
mod eval_cmd;
mod mcp;
mod secret;
mod serve;
mod setup;
mod update;

use clap::Parser;
use cli::{Cli, Command};
use update::UpdateArgs;

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
        Command::Secret(args) => secret::run(&args.command, config_path.as_deref()),
        Command::Config(args) => config_cmd::run(&args.command, config_path.as_deref()),
        Command::Doctor { online } => {
            let exit_code = doctor::run(online, config_path.as_deref()).await?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Command::Setup => setup::run_setup(config_path.as_deref()).await,
        Command::Model => setup::run_model(config_path.as_deref()).await,
        // M30: `bean update` không cần cấu hình — nguồn phát hành là hằng số
        // (D23.1), chỉ dùng mạng + đường dẫn binary đang chạy.
        Command::Update { check, no_restart } => {
            update::run(&UpdateArgs { check, no_restart }).await
        }
        // Eval: không cần `bean.toml` — runner tự dựng Config mặc định + ghi đè
        // theo task, mọi run nằm trong thư mục tạm (bean-eval).
        Command::Eval(args) => {
            let exit_code = eval_cmd::run(&args).await?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
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
