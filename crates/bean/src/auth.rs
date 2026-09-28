//! `bean auth` — tiện ích xác thực giao diện web.
//!
//! M1: chỉ có khung. M9 sẽ cài `set-password`: hash `argon2id` (argon2 0.6), ghi vào
//! `data.dir/auth.toml` với quyền `0600`, từ chối ghi đè nếu chưa xác nhận
//! (agents.md mục 15.7).

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bean_memory::{SqliteStore, Store};
use bean_types::config::Config;
use rpassword::prompt_password;

use crate::cli::{AuthArgs, AuthCommand, McpTokenCommand};

/// Chạy lệnh `auth`.
///
/// # Errors
/// Luôn trả lỗi ở M1.
pub async fn run(args: &AuthArgs, config_path: Option<&Path>) -> Result<()> {
    match &args.command {
        AuthCommand::SetPassword => {
            let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
            let data_dir = expand_tilde(&config.data.dir);
            let auth_path = data_dir.join("auth.toml");
            if auth_path.exists() && !io::stdin().is_terminal() {
                bail!("auth.toml đã tồn tại; cần TTY để xác nhận ghi đè");
            }
            if auth_path.exists() {
                print!("auth.toml đã tồn tại. Ghi đè? [y/N]: ");
                io::stdout().flush().context("flush stdout")?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).context("đọc xác nhận")?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    bail!("đã huỷ ghi đè auth.toml");
                }
            }
            if !io::stdin().is_terminal() {
                bail!("auth set-password cần TTY để nhập mật khẩu an toàn");
            }
            let password = prompt_password("Mật khẩu mới: ").context("đọc mật khẩu")?;
            let confirmation =
                prompt_password("Nhập lại mật khẩu: ").context("xác nhận mật khẩu")?;
            if password != confirmation {
                bail!("mật khẩu xác nhận không khớp");
            }
            if password.len() < 12 {
                bail!("mật khẩu phải có ít nhất 12 ký tự");
            }
            let database = Arc::new(
                SqliteStore::open(&data_dir.join("bean.db"))
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                    .context("mở database để vô hiệu session cũ")?,
            );
            let store: Arc<dyn Store> = database;
            bean_web::auth::set_password_and_revoke_sessions(&data_dir, &password, store)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .context("lưu hash và vô hiệu session đăng nhập cũ")?;
            println!(
                "Đã lưu hash argon2id trong {} với quyền 0600",
                auth_path.display()
            );
            Ok(())
        }
        AuthCommand::McpToken(args) => mcp_token(&args.command, config_path).await,
    }
}

/// Sinh token 256 bit, trả về dạng hex.
fn new_mcp_token() -> Result<String> {
    bean_core::mcp_server::new_token().map_err(|error| anyhow::anyhow!(error))
}

/// `auth mcp-token add|list|revoke` (M25).
///
/// Token thô **chỉ in một lần** và Bean chỉ lưu SHA-256 trong `mcp_clients` — đúng
/// cách `auth set-password` xử lý mật khẩu (mục 15.6).
async fn mcp_token(command: &McpTokenCommand, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("không tạo được data.dir {}", data_dir.display()))?;
    let store: Arc<dyn Store> = Arc::new(
        SqliteStore::open(&data_dir.join("bean.db"))
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("mở database token MCP")?,
    );
    match command {
        McpTokenCommand::Add { name } => {
            let name = name.trim();
            // Chặn ở tầng CLI: cấp token cho client không có trong `[[mcp_clients]]` là
            // vô nghĩa — `authenticate()` sẽ từ chối vì không tìm thấy trong cấu hình.
            let entry = config.mcp_client(name).with_context(|| {
                format!("client `{name}` chưa có trong [[mcp_clients]] của bean.toml")
            })?;
            let token = new_mcp_token()?;
            let ttl = config.mcp_server.token_ttl_hours;
            let expires = bean_core::mcp_server::auth::expiry_from(ttl);
            store
                .create_mcp_client(
                    &bean_core::mcp_server::hash_token(&token),
                    name,
                    &entry.role,
                    &chrono::Utc::now().to_rfc3339(),
                    &expires,
                )
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .context("lưu hash token")?;
            println!(
                "Token cho client `{name}` (role `{}`) — CHỈ HIỆN MỘT LẦN:",
                entry.role
            );
            println!("{token}");
            println!();
            println!("Lưu ngay vào file cấu hình MCP của IDE. Cấp lại sẽ vô hiệu token này.");
            if ttl == 0 {
                println!("Token không hết hạn (mcp_server.token_ttl_hours = 0).");
            } else {
                println!("Hết hạn lúc: {expires}");
            }
            println!();
            println!("Cho stdio, truyền qua biến môi trường:");
            println!("  BEAN_MCP_TOKEN={token}");
            println!("Cho HTTP, header: Authorization: Bearer {token}");
            Ok(())
        }
        McpTokenCommand::List => {
            let clients = store
                .list_mcp_clients()
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if clients.is_empty() {
                println!("Chưa có client MCP nào được cấp token.");
                return Ok(());
            }
            println!("{:<20} {:<20} {:<26} CẤP LÚC", "CLIENT", "ROLE", "HẾT HẠN");
            for client in clients {
                println!(
                    "{:<20} {:<20} {:<26} {}",
                    client.name,
                    client.role,
                    if client.expires_at.is_empty() {
                        "không".to_string()
                    } else {
                        client.expires_at.clone()
                    },
                    client.created_at
                );
            }
            Ok(())
        }
        McpTokenCommand::Revoke { name } => {
            let removed = store
                .delete_mcp_client(name.trim())
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if removed {
                println!("Đã thu hồi token của client `{name}`.");
                Ok(())
            } else {
                bail!("client `{name}` chưa có token nào được cấp")
            }
        }
    }
}

fn expand_tilde(path: &Path) -> std::path::PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}
