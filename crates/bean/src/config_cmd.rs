//! `bean config` — migrate key sai chỗ (M29) và bảng thiết lập thống nhất.
//!
//! * `migrate-secrets` — quét key ở bốn nguồn cũ, chép sang `secrets.env` (0600),
//!   dọn plaintext khỏi `bean.toml`/`voice-state.json`. Mặc định dry-run.
//! * `list`/`get`/`set`/`unset` — thao tác trên [`bean_settings::SETTINGS`]: khoá
//!   ngoài bảng bị từ chối, giá trị validate trước khi ghi, mọi lần ghi audit không
//!   kèm giá trị. Khoá tầng B (bảo mật) chỉ đổi được trong terminal tương tác.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bean_security::AuditLog;
use bean_settings::settings::{
    SETTINGS, SettingApply, SettingEntry, SettingSource, SettingsService, lookup, parse_value,
};
use bean_settings::{SecretSlotRegistry, SecretStore};
use bean_types::config::{Config, VoiceState};
use secrecy::SecretString;

use crate::chat::expand_tilde;
use crate::cli::ConfigCommandAction;

/// Một việc cần làm: chép `value` (lấy từ `source`) vào `slot`.
struct Migration {
    slot: String,
    value: String,
    source: String,
}

/// Chạy lệnh `config`.
///
/// # Errors
/// Lỗi nạp cấu hình, đọc file nguồn, hoặc lỗi đĩa khi ghi ở chế độ `--apply`.
pub fn run(command: &ConfigCommandAction, config_path: Option<&Path>) -> Result<()> {
    match command {
        ConfigCommandAction::MigrateSecrets {
            apply,
            from_env_file,
        } => migrate_secrets(*apply, from_env_file.as_deref(), config_path),
        ConfigCommandAction::List => config_list(config_path),
        ConfigCommandAction::Get { key } => config_get(key, config_path),
        ConfigCommandAction::Set { key, value } => config_set(key, value, config_path),
        ConfigCommandAction::Unset { key } => config_unset(key, config_path),
    }
}

/// Tìm khoá trong bảng; từ chối có hướng dẫn khi không có.
fn require_entry(key: &str) -> Result<&'static SettingEntry> {
    if let Some(entry) = lookup(key) {
        return Ok(entry);
    }
    // Tên ALL_CAPS không có dấu chấm gần như chắc chắn là biến môi trường — hướng
    // sang luồng secret thay vì để người dùng tự đoán.
    let looks_like_env_name = !key.contains('.')
        && key.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if looks_like_env_name {
        bail!(
            "`{key}` không phải khoá thiết lập — trông giống tên biến môi trường: dùng \
             `bean secret set {key}` để lưu giá trị (xem `bean secret list`)"
        );
    }
    bail!(
        "`{key}` không phải khoá thiết lập — xem `bean config list`. Cấu trúc TOML phức tạp \
         ([[roles]], [[infra_scope]], [[mcp_clients]], agent.user_roles…) không có trong bảng: \
         sửa trực tiếp bean.toml rồi restart"
    );
}

/// Cắt chuỗi theo ranh giới ký tự (không cắt giữa codepoint UTF-8).
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn config_list(config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let service = SettingsService::new(&config, config_path);

    println!(
        "{:<38} {:<3} {:<8} {:<17} {:<34} NGUỒN",
        "KHOÁ", "TẦNG", "ÁP DỤNG", "NƠI LƯU", "GIÁ TRỊ"
    );
    let mut entries: Vec<&SettingEntry> = SETTINGS.iter().collect();
    entries.sort_by_key(|entry| entry.path);
    for entry in entries {
        let read = service.get(entry);
        println!(
            "{:<38} {:<3} {:<8} {:<17} {:<34} {}",
            entry.path,
            entry.tier.short(),
            entry.apply.label(),
            entry.store.label(),
            truncate_chars(&read.value, 32),
            read.source.label(),
        );
    }
    println!();
    println!("Tầng: A = đổi được từ UI + CLI · B = chỉ CLI tương tác (bảo mật).");
    println!("Secret (key API) không nằm trong bảng này — dùng `bean secret list`.");
    Ok(())
}

fn config_get(key: &str, config_path: Option<&Path>) -> Result<()> {
    let entry = require_entry(key)?;
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let service = SettingsService::new(&config, config_path);
    let read = service.get(entry);
    println!(
        "{} = {} [nguồn: {}]",
        entry.path,
        read.value,
        read.source.label()
    );
    println!(
        "kiểu: {} · tầng: {} · áp dụng: {} · lưu ở: {}",
        entry.kind.label(),
        entry.tier.label(),
        entry.apply.label(),
        entry.store.label()
    );
    println!("{}", entry.desc_vi);
    Ok(())
}

fn config_set(key: &str, value: &str, config_path: Option<&Path>) -> Result<()> {
    let entry = require_entry(key)?;
    // Parse sớm: giá trị sai kiểu thì dừng trước khi đụng đĩa.
    let parsed = parse_value(entry, value)?;

    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let service = SettingsService::new(&config, config_path);
    let before = service.get(entry);

    if entry.tier.cli_only() {
        if !io::stdin().is_terminal() {
            bail!(
                "`{}` là khoá tầng B — chỉ đổi được trong terminal tương tác (chống script/cron \
                 sửa cấu hình bảo mật)",
                entry.path
            );
        }
        println!("cũ: {}", before.value);
        println!("mới: {}", parsed.display());
        if !confirm_prompt("Ghi thay đổi? [y/N]: ")? {
            println!("Đã huỷ — không ghi gì.");
            return Ok(());
        }
    }

    let data_dir = expand_tilde(&config.data.dir);
    let audit = AuditLog::open(&data_dir).context("mở audit log thất bại")?;
    let outcome = service.set(entry, parsed, &audit, "cli:local")?;
    println!(
        "Đã ghi {} (nguồn: {} → {}).",
        entry.path,
        outcome.old_source.label(),
        outcome.new_source.label()
    );
    print_apply_banner(outcome.apply);
    Ok(())
}

fn config_unset(key: &str, config_path: Option<&Path>) -> Result<()> {
    let entry = require_entry(key)?;
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let service = SettingsService::new(&config, config_path);
    let before = service.get(entry);

    if before.source == SettingSource::Default {
        println!(
            "`{}` đã ở giá trị mặc định — không có gì để bỏ.",
            entry.path
        );
        return Ok(());
    }
    if entry.tier.cli_only() {
        if !io::stdin().is_terminal() {
            bail!(
                "`{}` là khoá tầng B — chỉ đổi được trong terminal tương tác",
                entry.path
            );
        }
        if !confirm_prompt(&format!(
            "Bỏ ghi đè `{}` (về giá trị mặc định)? [y/N]: ",
            entry.path
        ))? {
            println!("Đã huỷ — không ghi gì.");
            return Ok(());
        }
    }

    let data_dir = expand_tilde(&config.data.dir);
    let audit = AuditLog::open(&data_dir).context("mở audit log thất bại")?;
    let outcome = service.unset(entry, &audit, "cli:local")?;
    println!(
        "Đã bỏ ghi đè {} (nguồn: {} → {}).",
        entry.path,
        outcome.old_source.label(),
        outcome.new_source.label()
    );
    print_apply_banner(outcome.apply);
    Ok(())
}

/// Nhắc sau khi ghi: khoá "hot" không tự nạp lại file ở tiến trình đang chạy;
/// khoá restart phải khởi động lại hẳn.
fn print_apply_banner(apply: SettingApply) {
    match apply {
        SettingApply::Hot => {
            println!("Khoá hot — tiến trình đang chạy không tự nạp lại file.");
        }
        SettingApply::Restart => {
            println!("CẦN RESTART — chạy `scripts/beanctl restart`.");
        }
    }
}

fn migrate_secrets(
    apply: bool,
    from_env_file: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let registry = SecretSlotRegistry::from_config(&config);
    let snapshot = store.snapshot().unwrap_or_default();

    let mut migrations: Vec<Migration> = Vec::new();
    // 1) Key trong `bean.toml` ([llm].api_key + [[llm.providers]].api_key).
    if let Some(key) = config
        .llm
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        migrations.push(Migration {
            slot: slot_for(&config.llm.api_key_env, "default"),
            value: key.to_string(),
            source: "bean.toml [llm].api_key".to_string(),
        });
    }
    for profile in &config.llm.providers {
        if let Some(key) = profile
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            migrations.push(Migration {
                slot: slot_for(&profile.api_key_env, &profile.name),
                value: key.to_string(),
                source: format!("bean.toml [[llm.providers]] `{}`", profile.name),
            });
        }
    }
    // 2) Key trong `voice-state.json` (nhập từ UI trước M29).
    let voice_state_path = data_dir.join("voice-state.json");
    let saved_voice = VoiceState::load(&voice_state_path);
    if let Some(state) = &saved_voice
        && let Some(key) = state
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
    {
        migrations.push(Migration {
            slot: config.voice.api_key_env.trim().to_string(),
            value: key.to_string(),
            source: format!("`{}`", voice_state_path.display()),
        });
    }
    // 3) File env bổ sung (`--from-env-file`, mặc định `~/.config/bean/env`).
    let env_file = from_env_file
        .map(Path::to_path_buf)
        .unwrap_or_else(default_env_file);
    if env_file.is_file() {
        let raw = std::fs::read_to_string(&env_file)
            .with_context(|| format!("đọc {}", env_file.display()))?;
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if registry.slot(name).is_none() || value.is_empty() {
                // Tên ngoài slot (PATH, DISPLAY…) hoặc giá trị rỗng: bỏ qua.
                continue;
            }
            migrations.push(Migration {
                slot: name.to_string(),
                value: value.to_string(),
                source: format!("`{}`", env_file.display()),
            });
        }
    }

    if migrations.is_empty() {
        println!("Không tìm thấy key nào sai chỗ — không cần migrate.");
        return Ok(());
    }

    // Gộp trùng slot: cùng giá trị ⇒ một mục; khác giá trị ⇒ ghi nhận xung đột.
    let mut plan: Vec<Migration> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for migration in migrations {
        if let Some(existing) = plan.iter_mut().find(|entry| entry.slot == migration.slot) {
            if existing.value != migration.value {
                conflicts.push(format!(
                    "`{}` có giá trị khác nhau giữa {} và {}",
                    migration.slot, existing.source, migration.source
                ));
            }
            continue;
        }
        plan.push(migration);
    }

    println!("Kế hoạch migrate ({} slot):", plan.len());
    for entry in &plan {
        let state = if !snapshot.contains(&entry.slot) {
            "chưa có → chép sang"
        } else if snapshot.lookup_str(&entry.slot).as_deref() == Some(entry.value.as_str()) {
            "đã có sẵn (giống nhau) — bỏ qua"
        } else {
            "đã có sẵn (KHÁC giá trị) — sẽ hỏi/skip"
        };
        println!("  {:<36} ← {:<44} [{state}]", entry.slot, entry.source);
    }
    for conflict in &conflicts {
        println!("  XUNG ĐỘT: {conflict}");
    }
    if !conflicts.is_empty() {
        println!("  (xung đột: giá trị đầu tiên trong kế hoạch thắng trừ khi bạn chọn khác)");
    }
    if !apply {
        println!();
        println!("Đây là dry-run — chạy lại với `--apply` để ghi thật.");
        return Ok(());
    }

    // ---- APPLY ----
    let interactive = io::stdin().is_terminal();
    let mut applied = 0;
    let mut skipped = 0;
    let mut touched_toml = false;
    let mut touched_voice = false;
    for entry in &plan {
        let decision = if !snapshot.contains(&entry.slot) {
            true
        } else if snapshot.lookup_str(&entry.slot).as_deref() == Some(entry.value.as_str()) {
            false
        } else if !interactive {
            eprintln!(
                "bỏ qua `{}`: secrets.env đã có giá trị khác và không có TTY để xác nhận",
                entry.slot
            );
            skipped += 1;
            false
        } else {
            confirm_prompt(&format!(
                "secrets.env đã có giá trị KHÁC cho `{}`. Ghi đè? [y/N]: ",
                entry.slot
            ))?
        };
        if !decision {
            continue;
        }
        // Registry tạm cho slot này: tên sinh động (`LLM_<TÊN>_API_KEY`) chưa có
        // trong registry dẫn xuất từ config.
        let slot_registry = SecretSlotRegistry::from_names(&[entry.slot.as_str()]);
        store
            .set(
                &slot_registry,
                &entry.slot,
                SecretString::from(entry.value.clone()),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .with_context(|| format!("ghi slot `{}` thất bại", entry.slot))?;
        if entry.source.contains("bean.toml") {
            touched_toml = true;
        }
        if entry.source.contains("voice-state") {
            touched_voice = true;
        }
        applied += 1;
    }

    // Backup + dọn key khỏi nguồn cũ — chỉ khi có slot nào thực sự được ghi.
    if applied > 0 {
        if touched_toml
            && let Some(path) = config_path
            && path.is_file()
        {
            backup(path)?;
            let removed = bean_types::config::remove_stored_api_keys(path)
                .with_context(|| format!("dọn api_key khỏi {}", path.display()))?;
            println!("Đã xoá {removed} trường api_key khỏi {}.", path.display());
        }
        if touched_voice
            && let Some(state) = saved_voice
            && state
                .api_key
                .as_deref()
                .map(str::trim)
                .is_some_and(|key| !key.is_empty())
        {
            backup(&voice_state_path)?;
            let mut cleaned = state;
            cleaned.api_key = None;
            VoiceState::save(&voice_state_path, &cleaned)
                .with_context(|| format!("xoá key khỏi {}", voice_state_path.display()))?;
            println!("Đã xoá key khỏi {}.", voice_state_path.display());
        }
    }
    println!();
    println!("Xong: {applied} slot được ghi, {skipped} bị bỏ qua.");
    println!("Khởi động lại Bean để mọi thành phần đọc được key mới.");
    Ok(())
}

/// Tên slot cho một nguồn LLM: dùng `api_key_env` nếu có, sinh `LLM_<TÊN>_API_KEY`
/// nếu nguồn chưa khai báo (giống luồng UI M29).
fn slot_for(api_key_env: &str, profile_name: &str) -> String {
    let name = api_key_env.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    let sanitized: String = profile_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .to_ascii_uppercase();
    format!("LLM_{sanitized}_API_KEY")
}

fn default_env_file() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join(".config/bean/env"),
        Err(_) => PathBuf::from(".config/bean/env"),
    }
}

/// Copy file thành `<tên>.pre-m29` với quyền 0600.
fn backup(path: &Path) -> Result<()> {
    let backup_path = path.with_extension("pre-m29");
    std::fs::copy(path, &backup_path)
        .with_context(|| format!("backup {} → {}", path.display(), backup_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&backup_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0600 cho {}", backup_path.display()))?;
    }
    println!("Đã backup {} → {}", path.display(), backup_path.display());
    Ok(())
}

/// Hỏi y/N qua TTY; mọi câu trả lời khác "y"/"yes" coi như từ chối.
fn confirm_prompt(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    io::stdout().flush().context("flush stdout")?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).context("đọc xác nhận")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
