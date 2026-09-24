//! `BeanAgent auth` — tiện ích xác thực giao diện web.
//!
//! M1: chỉ có khung. M9 sẽ cài `set-password`: hash `argon2id` (argon2 0.6), ghi vào
//! `data.dir/auth.toml` với quyền `0600`, từ chối ghi đè nếu chưa xác nhận
//! (agents.md mục 15.7).

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use beanagent_memory::{SqliteStore, Store};
use beanagent_types::config::Config;
use rpassword::prompt_password;

use crate::cli::{AuthArgs, AuthCommand};

/// Chạy lệnh `auth`.
///
/// # Errors
/// Luôn trả lỗi ở M1.
pub async fn run(args: &AuthArgs, config_path: Option<&Path>) -> Result<()> {
    match args.command {
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
                SqliteStore::open(&data_dir.join("beanagent.db"))
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                    .context("mở database để vô hiệu session cũ")?,
            );
            let store: Arc<dyn Store> = database;
            beanagent_web::auth::set_password_and_revoke_sessions(&data_dir, &password, store)
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .context("lưu hash và vô hiệu session đăng nhập cũ")?;
            println!(
                "Đã lưu hash argon2id trong {} với quyền 0600",
                auth_path.display()
            );
            Ok(())
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
