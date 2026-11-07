use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::{Context, Result, bail};
use bean_core::mcp_server::auth::{hash_token, new_token};
use bean_memory::SqliteStore;
use bean_memory::traits::McpClientStore;
use bean_types::Config;
use bean_web::auth::set_password as web_set_password;
use chrono::Utc;
use std::sync::Arc;

use crate::chat::{expand_tilde, store_path};
use crate::cli::{AuthArgs, AuthCommand, McpTokenCommand};

/// Ghi hash mật khẩu vào `auth.toml` dưới dạng `AuthFile` TOML kèm `password_changed_at`
/// để server thu hồi session cũ qua mốc thời gian (Q1). CLI không mở DB.
pub fn set_password(data_dir: &Path, password: &str) -> Result<()> {
    if password.len() < 12 {
        bail!("mật khẩu phải có ít nhất 12 ký tự");
    }
    let changed_at = Utc::now().to_rfc3339();
    web_set_password(data_dir, password, Some(&changed_at))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "Đã lưu hash mật khẩu vào {} (quyền 0600, mốc thu hồi {changed_at}).",
        data_dir.join("auth.toml").display()
    );
    Ok(())
}

/// Chạy lệnh `auth` (set-password và mcp-token).
pub async fn run(args: &AuthArgs, config_path: Option<&Path>) -> Result<()> {
    match &args.command {
        AuthCommand::SetPassword => {
            if !io::stdin().is_terminal() {
                bail!("`auth set-password` cần TTY để nhập ẩn — chạy trong terminal tương tác");
            }
            let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
            let data_dir = expand_tilde(&config.data.dir);
            let password = rpassword::prompt_password("Nhập mật khẩu web (ít nhất 12 ký tự): ")
                .context("đọc mật khẩu")?;
            let confirmation =
                rpassword::prompt_password("Nhập lại để xác nhận: ").context("đọc xác nhận")?;
            if password != confirmation {
                bail!("mật khẩu xác nhận không khớp");
            }
            set_password(&data_dir, &password)?;
            println!(
                "Đã lưu hash mật khẩu vào {} (quyền 0600, mốc thu hồi).",
                data_dir.join("auth.toml").display()
            );
            Ok(())
        }
        AuthCommand::McpToken(mcp_args) => run_mcp_token(mcp_args, config_path).await,
    }
}

/// Quản lý token client MCP (`bean auth mcp-token add|list|revoke`).
async fn run_mcp_token(args: &crate::cli::McpTokenArgs, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);
    std::fs::create_dir_all(&data_dir).with_context(|| format!("tạo {}", data_dir.display()))?;
    let store = Arc::new(
        SqliteStore::open(&store_path(&config))
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("không mở được SQLite store")?,
    );

    match &args.command {
        McpTokenCommand::Add { name } => {
            if config.mcp_client(name).is_none() {
                bail!("client `{name}` không có trong [[mcp_clients]] — thêm vào bean.toml trước");
            }
            let token = new_token().map_err(|e| anyhow::anyhow!(e))?;
            let token_hash = hash_token(&token);
            let now = chrono::Utc::now().to_rfc3339();
            // Không hạn (expires_at rỗng) — client quản lý lượt vòng đời của chính mình.
            store
                .create_mcp_client(&token_hash, name, "mcp-client", &now, "")
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            println!("Token cho client `{name}` (chỉ hiển thị một lần):");
            println!("  {token}");
            println!("Lưu token vào Cline/Cursor/OpenCode trong mục MCP authentication.");
            Ok(())
        }
        McpTokenCommand::List => {
            let clients = store
                .list_mcp_clients()
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if clients.is_empty() {
                println!("Chưa có client MCP nào được cấp token.");
            } else {
                println!("{:<30} {:<28} {:<20}", "TÊN CLIENT", "VÀO LÚC", "HẾT HẠN");
                for client in &clients {
                    let expires = if client.expires_at.is_empty() {
                        "không".to_string()
                    } else {
                        client.expires_at.clone()
                    };
                    println!(
                        "{:<30} {:<28} {:<20}",
                        client.name, client.created_at, expires
                    );
                }
            }
            Ok(())
        }
        McpTokenCommand::Revoke { name } => {
            let removed = store
                .delete_mcp_client(name)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if removed {
                println!("Đã thu hồi token của client `{name}`.");
            } else {
                println!("Không tìm thấy client `{name}`.");
            }
            Ok(())
        }
    }
}
