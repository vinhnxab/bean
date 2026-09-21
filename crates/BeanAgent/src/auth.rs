//! `BeanAgent auth` — tiện ích xác thực giao diện web.
//!
//! M1: chỉ có khung. M9 sẽ cài `set-password`: hash `argon2id` (argon2 0.6), ghi vào
//! `data.dir/auth.toml` với quyền `0600`, từ chối ghi đè nếu chưa xác nhận
//! (agents.md mục 15.7).

use anyhow::{Result, bail};

use crate::cli::{AuthArgs, AuthCommand};

/// Chạy lệnh `auth`.
///
/// # Errors
/// Luôn trả lỗi ở M1.
pub fn run(args: &AuthArgs) -> Result<()> {
    match args.command {
        AuthCommand::SetPassword => bail!(
            "`auth set-password` chưa được cài đặt (M9): sẽ ghi hash argon2id vào data.dir/auth.toml với quyền 0600"
        ),
    }
}
