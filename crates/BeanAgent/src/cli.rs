//! Định nghĩa CLI bằng `clap` (agents.md mục 18: `chat`, `serve`, `auth`).

use std::path::{Path, PathBuf};

use beanagent_types::config::DEFAULT_CONFIG_FILE;
use clap::{Args, Parser, Subcommand};

/// Trợ lý AI cá nhân tự host: một binary Rust (CLI + web + Telegram).
#[derive(Debug, Parser)]
#[command(name = "BeanAgent", version, about, long_about = None)]
pub struct Cli {
    /// File cấu hình TOML. Mặc định dùng `./BeanAgent.toml` nếu file này tồn tại.
    #[arg(long, global = true, env = "BEANAGENT_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Lệnh cần chạy.
    #[command(subcommand)]
    pub command: Command,
}

/// Các lệnh con.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// REPL trên terminal (không cần web, không cần Node).
    Chat(ChatArgs),

    /// Chạy web + Telegram + scheduler trong một tiến trình.
    Serve(ServeArgs),

    /// Tiện ích xác thực cho giao diện web.
    Auth(AuthArgs),
}

/// Tham số của `chat`.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Dùng kịch bản JSON làm provider giả thay vì gọi API thật.
    #[arg(long, value_name = "FILE")]
    pub fake_llm: Option<PathBuf>,
}

/// Tham số của `serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Dùng kịch bản JSON làm provider giả (dùng cho test end-to-end).
    #[arg(long, value_name = "FILE")]
    pub fake_llm: Option<PathBuf>,
}

/// Tham số của `auth`.
#[derive(Debug, Args)]
pub struct AuthArgs {
    /// Lệnh con của `auth`.
    #[command(subcommand)]
    pub command: AuthCommand,
}

/// Các lệnh con của `auth`.
#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Đặt mật khẩu cho giao diện web (lưu hash argon2id vào `data.dir/auth.toml`).
    SetPassword,
}

impl Cli {
    /// File cấu hình hiệu dụng: `--config`, hoặc `./BeanAgent.toml` nếu tồn tại.
    #[must_use]
    pub fn config_path(&self) -> Option<&Path> {
        if let Some(path) = self.config.as_deref() {
            return Some(path);
        }
        let default = Path::new(DEFAULT_CONFIG_FILE);
        if default.is_file() {
            Some(default)
        } else {
            None
        }
    }
}
