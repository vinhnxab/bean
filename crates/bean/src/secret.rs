//! `bean secret` — quản lý kho secret thống nhất `data.dir/secrets.env` (M29).
//!
//! Ba lệnh con: `set` (nhập ẩn TTY hoặc `--stdin`), `list` (nguồn mỗi slot, không
//! bao giờ in giá trị), `unset`. Tên slot phải được khai báo trong `bean.toml`
//! qua các trường `*_env` — [`bean_settings::SecretSlotRegistry`] là bộ cửa duy
//! nhất, không ai ghi được tên lạ vào `secrets.env`.

use std::io::{self, IsTerminal};
use std::path::Path;

use anyhow::{Context, Result, bail};
use bean_settings::{SecretSlotRegistry, SecretStore};
use bean_types::config::Config;
use rpassword::prompt_password;
use secrecy::SecretString;

use crate::chat::expand_tilde;
use crate::cli::SecretCommandAction;

/// Chạy lệnh `secret`.
///
/// # Errors
/// Lỗi nạp cấu hình, tên slot lạ, hoặc lỗi đĩa khi ghi/xoá `secrets.env`.
pub fn run(command: &SecretCommandAction, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let registry = SecretSlotRegistry::from_config(&config);
    match command {
        SecretCommandAction::Set { name, stdin } => secret_set(&store, &registry, name, *stdin),
        SecretCommandAction::List => secret_list(&store, &registry),
        SecretCommandAction::Unset { name } => secret_unset(&store, &registry, name),
    }
}

fn secret_set(
    store: &SecretStore,
    registry: &SecretSlotRegistry,
    name: &str,
    stdin: bool,
) -> Result<()> {
    let name = name.trim();
    if registry.slot(name).is_none() {
        bail!(
            "slot `{name}` chưa được khai báo trong bean.toml — thêm trường `*_env` tương ứng trước (ví dụ llm.api_key_env)"
        );
    }
    let value = if stdin {
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .context("đọc giá trị từ stdin")?;
        // Chỉ bỏ hậu tố dòng (LF/CRLF), giữ nguyên mọi ký tự khác của key.
        line.trim_end_matches(['\r', '\n']).to_string()
    } else {
        if !io::stdin().is_terminal() {
            bail!("`secret set` cần TTY để nhập ẩn — dùng `--stdin` khi chạy không tương tác");
        }
        let value = prompt_password(format!("Giá trị cho {name}: ")).context("đọc giá trị")?;
        let confirmation = prompt_password("Nhập lại để xác nhận: ").context("đọc xác nhận")?;
        if value != confirmation {
            bail!("giá trị xác nhận không khớp");
        }
        value
    };
    store
        .set(registry, name, SecretString::from(value))
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .with_context(|| format!("ghi slot `{name}` thất bại"))?;
    println!(
        "Đã lưu `{name}` vào {} (0600). Nếu biến môi trường cùng tên cũng đang đặt thì env vẫn thắng.",
        store.path().display()
    );
    Ok(())
}

fn secret_list(store: &SecretStore, registry: &SecretSlotRegistry) -> Result<()> {
    let snapshot = match store.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            // File hỏng không chặn việc liệt kê — chỉ báo nguồn chưa đọc được.
            eprintln!("cảnh báo: không đọc được secrets.env: {error}");
            bean_settings::SecretSnapshot::default()
        }
    };
    if registry.is_empty() {
        println!("Cấu hình chưa khai báo slot secret nào (trường `*_env`).");
        return Ok(());
    }
    println!("{:<34} {:<16} {:<22} NGUỒN", "SLOT", "NHÓM", "DÙNG BỞI");
    for slot in registry.slots() {
        let env_set = std::env::var(&slot.name).is_ok_and(|value| !value.trim().is_empty());
        let in_file = snapshot.contains(&slot.name);
        let source = match (env_set, in_file) {
            (true, true) => "env (che secrets.env)",
            (true, false) => "env",
            (false, true) => "secrets.env",
            (false, false) => "chưa đặt",
        };
        println!(
            "{:<34} {:<16} {:<22} {}",
            slot.name,
            slot.group.label(),
            slot.used_by.join(", "),
            source
        );
    }
    Ok(())
}

fn secret_unset(store: &SecretStore, registry: &SecretSlotRegistry, name: &str) -> Result<()> {
    let name = name.trim();
    if registry.slot(name).is_none() {
        bail!("slot `{name}` chưa được khai báo trong bean.toml");
    }
    let removed = store
        .remove(registry, name)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .with_context(|| format!("xoá slot `{name}` thất bại"))?;
    if removed {
        println!("Đã xoá `{name}` khỏi {}.", store.path().display());
    } else {
        println!("`{name}` không có trong secrets.env (không có gì để xoá).");
    }
    Ok(())
}
