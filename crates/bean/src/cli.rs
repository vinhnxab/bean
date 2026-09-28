//! Định nghĩa CLI bằng `clap` (agents.md mục 18: `chat`, `serve`, `auth`).

use std::path::{Path, PathBuf};

use bean_types::config::DEFAULT_CONFIG_FILE;
use clap::{Args, Parser, Subcommand};

/// Trợ lý AI cá nhân tự host: một binary Rust (CLI + web + Telegram).
#[derive(Debug, Parser)]
#[command(name = "bean", version, about, long_about = None)]
pub struct Cli {
    /// File cấu hình TOML. Mặc định dùng `./bean.toml` nếu file này tồn tại.
    #[arg(long, global = true, env = "BEAN_CONFIG", value_name = "FILE")]
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

    /// Chạy web + Telegram trong một tiến trình.
    Serve(ServeArgs),

    /// Tiện ích xác thực cho giao diện web.
    Auth(AuthArgs),

    /// Chạy Bean ở chế độ **MCP server read-only** (M25) cho Cline/Cursor/OpenCode…
    Mcp(McpArgs),
}

/// Tham số của `mcp`.
#[derive(Debug, Args)]
pub struct McpArgs {
    /// Lệnh con của `mcp`.
    #[command(subcommand)]
    pub command: McpCommand,
}

/// Các lệnh con của `mcp`.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Chạy MCP server. Mặc định `stdio` (Bean cùng máy với IDE);
    /// `--http` dùng streamable-HTTP/SSE (Bean chạy remote, sau reverse proxy).
    Serve {
        /// Dùng transport streamable-HTTP thay vì stdio.
        #[arg(long)]
        http: bool,
    },
}

/// Tham số của `chat`.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// Dùng kịch bản JSON làm provider giả thay vì gọi API thật.
    #[arg(long, value_name = "FILE")]
    pub fake_llm: Option<PathBuf>,

    /// Ghi đè thư mục workspace (`[agent] workspace`) — tiện cho demo/test.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,
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

    /// Quản lý token của client MCP (M25): `add` / `list` / `revoke`.
    McpToken(McpTokenArgs),
}

/// Tham số của `auth mcp-token`.
#[derive(Debug, Args)]
pub struct McpTokenArgs {
    /// Lệnh con của `mcp-token`.
    #[command(subcommand)]
    pub command: McpTokenCommand,
}

/// Các lệnh con của `auth mcp-token`.
#[derive(Debug, Subcommand)]
pub enum McpTokenCommand {
    /// Cấp token mới cho một client (in token thô **một lần duy nhất**).
    Add {
        /// Tên client, phải có trong `[[mcp_clients]]`.
        name: String,
    },
    /// Liệt kê client đã được cấp token.
    List,
    /// Thu hồi token của một client.
    Revoke {
        /// Tên client cần thu hồi.
        name: String,
    },
}

impl Cli {
    /// File cấu hình hiệu dụng: `--config`, hoặc `./bean.toml` nếu tồn tại.
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
