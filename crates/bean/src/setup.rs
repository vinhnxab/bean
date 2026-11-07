//! `bean setup` — wizard tương tác ban đầu (idempotent) và `bean model`.
//!
//! # Nguyên tắc
//!
//! * Không bao giờ nhận secret qua tham số dòng lệnh (lọt shell history, `ps`…).
//!   `setup` hỏi key khi cần thiết, nhập ẩn qua TTY; không TTY ⇒ bỏ qua bước đó
//!   với cảnh báo, không lỗi hẳn.
//! * Mọi ghi cấu hình đều qua `SettingsService` (validate trước khi ghi, audit
//!   không kèm giá trị); secret qua `SecretStore`; không bao giờ in giá trị secret.

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use crate::auth;
use crate::chat::{build_audit, expand_tilde};
use anyhow::{Context, Result, bail};
use bean_llm::{ChatRequest, build_provider};
use bean_security::AuditLog;
use bean_settings::{SecretSlotRegistry, SecretStore, SettingsService, parse_value};
use bean_types::Message;
use bean_types::config::{Config, SettingsState, WebSearchProvider};
use secrecy::SecretString;

/// Nhập một dòng từ TTY; Enter rỗng = bỏ qua.
fn read_line(prompt: &str) -> Result<Option<String>> {
    if !io::stdin().is_terminal() {
        return Ok(None);
    }
    print!("{prompt} ");
    io::stdout().flush().context("flush stdout")?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("đọc nhập từ terminal")?;
    let line = line.trim().to_string();
    Ok(if line.is_empty() { None } else { Some(line) })
}

/// Nhập giá trị ẩn từ TTY (không được nhận qua argv).
fn read_secret(prompt: &str) -> Result<Option<String>> {
    if !io::stdin().is_terminal() {
        return Ok(None);
    }
    rpassword::prompt_password(prompt)
        .context("đọc giá trị ẩn")
        .map(Some)
}

/// Nhắc xác nhận `y/N`; Enter = bỏ qua.
fn confirm(prompt: &str) -> Result<bool> {
    loop {
        match read_line(&format!("{prompt} [y/N]: "))? {
            None => return Ok(false),
            Some(answer) => match answer.to_ascii_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" | "" => return Ok(false),
                _ => println!("Nhập `y` hoặc `N`."),
            },
        }
    }
}

/// Chọn một mục từ danh sách (số thứ tự); Enter rỗng hoặc `0` = bỏ qua.
fn choose(prompt: &str, items: &[(String, String)]) -> Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }
    println!("{prompt}");
    for (index, (_, label)) in items.iter().enumerate() {
        println!("  {}. {}", index + 1, label);
    }
    println!("  0. Bỏ qua");
    loop {
        match read_line("Chọn:")? {
            None => return Ok(None),
            Some(answer) => {
                if answer == "0" {
                    return Ok(None);
                }
                match answer.parse::<usize>() {
                    Ok(n) if n >= 1 && n <= items.len() => return Ok(Some(n - 1)),
                    _ => println!("Nhập lại số từ 0 đến {}", items.len()),
                }
            }
        }
    }
}

/// Ghi một khoá bảng thiết lập vào `bean.toml` (validate trước khi ghi, audit
/// không kèm giá trị).
fn set_toml(service: &SettingsService, audit: &AuditLog, key: &str, value: &str) -> Result<()> {
    let entry = bean_settings::lookup(key)
        .with_context(|| format!("`{key}` không có trong bảng thiết lập"))?;
    let value = parse_value(entry, value)?;
    service
        .set(entry, value, audit, "cli:setup")
        .with_context(|| format!("ghi `{key}` thất bại"))?;
    Ok(())
}

/// Trả khóa API của một slot đã thiết lập (env > secrets.env), chưa thiết lập ⇒ None.
fn slot_value(config: &Config, _registry: &SecretSlotRegistry, env_name: &str) -> Option<String> {
    let store = SecretStore::new(expand_tilde(&config.data.dir).join("secrets.env"));
    let snapshot = store.snapshot().ok()?;
    let get_env = bean_settings::env_first_lookup(Some(&snapshot));
    get_env(env_name)
}

/// Lưu `settings-state.json` (profile + model + voice, áp dụng khi khởi động lại Bean).
fn save_settings_state(data_dir: &Path, profile: &str, model: &str) -> Result<()> {
    let state = SettingsState {
        llm_profile: profile.to_string(),
        llm_model: model.to_string(),
        voice_enabled: false,
        voice_provider: None,
        voice_language: None,
        voice_tts_voice_id: None,
        voice_tts_voice_vi: None,
    };
    SettingsState::save(
        &data_dir.join(bean_types::config::SETTINGS_STATE_FILE),
        &state,
    )
    .context("ghi settings-state.json thất bại")?;
    Ok(())
}

/// Ping kết nối thử LLM (tối đa 1 token đầu ra) — cảnh báo nếu thất bại, không
/// chặn wizard.
async fn probe_llm(config: &Config, key: Option<&SecretString>) {
    let Ok(provider) = build_provider(&config.llm, key.cloned()) else {
        println!("⚠ Không dựng được nhà cung cấp LLM: kiểm tra cấu hình và API key");
        return;
    };
    let messages = [Message::user("ping")];
    let request = ChatRequest {
        system: "Bean setup kết nối thử.",
        messages: &messages,
        tools: &[],
        max_tokens: 1,
    };
    match provider.chat(request).await {
        Ok(response) => println!(
            "✓ Kết nối OK ({} in / {} out tokens)",
            response.usage.input_tokens, response.usage.output_tokens
        ),
        Err(error) => println!("⚠ Kết nối thử thất bại: {error} (bước này có thể bỏ qua)"),
    }
}

/// Bước 1: nhà cung cấp LLM + model + API key.
async fn setup_provider(
    config: &mut Config,
    store: &SecretStore,
    registry: &SecretSlotRegistry,
    service: &SettingsService,
    audit: &AuditLog,
    _config_path: Option<&Path>,
) -> Result<()> {
    println!("\n=== Bước 1/5: Nhà cung cấp LLM & model ===");
    let profiles = config.llm.profiles();
    let current = config.llm.active_profile_name();
    let items: Vec<(String, String)> = profiles
        .iter()
        .map(|profile| {
            let active = if profile.name == current {
                " (đang dùng)"
            } else {
                ""
            };
            (
                profile.name.clone(),
                format!(
                    "{} · {} · {} {}",
                    profile.provider.as_str(),
                    profile.model,
                    if profile.api_key_env.is_empty() {
                        "không cần key".to_string()
                    } else {
                        format!("(key: {})", profile.api_key_env)
                    },
                    active
                ),
            )
        })
        .collect();
    let Some(idx) = choose("Chọn nhà cung cấp/mô hình (profile):", &items)? else {
        println!("Bỏ qua bước 1.");
        return Ok(());
    };
    let profile = &profiles[idx];

    // Đổi nhà cung cấp: ghi các trường [llm] theo thứ tự an toàn (allowed_models
    // trước, model sau) để mỗi lần ghi đều đi qua `Config::validate()`.
    if profile.name != current {
        for (key, value) in [
            (
                "llm.allowed_models",
                serde_json::to_string(&profile.allowed_models).with_context(|| {
                    format!("serialize allowed_models cho profile `{}", profile.name)
                })?,
            ),
            ("llm.base_url", profile.base_url.clone().unwrap_or_default()),
            ("llm.max_tokens", profile.max_tokens.to_string()),
            ("llm.api_key_env", profile.api_key_env.clone()),
            ("llm.provider", profile.provider.as_str().to_string()),
            ("llm.model", profile.model.clone()),
        ] {
            set_toml(service, audit, key, &value)?;
        }
        config.llm.apply_profile(profile);
        save_settings_state(
            &expand_tilde(&config.data.dir),
            &profile.name,
            &profile.model,
        )?;
        println!(
            "Đã chuyển sang profile `{}` và lưu llm-state.json.",
            profile.name
        );
    }

    // Chọn model (hoặc nhập tên tuỳ ý).
    let allowed = config.llm.effective_allowed_models();
    let model = if allowed.is_empty() {
        match read_line("Nhập tên model:")? {
            Some(model) if !model.is_empty() => model,
            _ => {
                println!("Bỏ qua chọn model.");
                profile.model.clone()
            }
        }
    } else {
        let items: Vec<(String, String)> = allowed
            .iter()
            .map(|model| (model.clone(), model.clone()))
            .collect();
        match choose("Chọn model:", &items)? {
            Some(idx) => items[idx].1.clone(),
            None => match read_line("Hoặc nhập tên model khác:")? {
                Some(model) if !model.is_empty() => model,
                _ => {
                    println!("Bỏ qua chọn model.");
                    profile.model.clone()
                }
            },
        }
    };
    if model != config.llm.model {
        set_toml(service, audit, "llm.model", &model)?;
        save_settings_state(
            &expand_tilde(&config.data.dir),
            config.llm.active_profile_name(),
            &model,
        )?;
        config.llm.model = model.clone();
        println!("Đã lưu model `{model}` vào settings-state.json.");
    }

    // API key (nếu nhà cung cấp cần). Probe trước khi move key vào store.
    let env_name = config.llm.api_key_env.clone();
    if env_name.trim().is_empty() {
        println!("Provider này không cần API key.");
        return Ok(());
    }
    let already_set = slot_value(config, registry, &env_name).is_some();
    if already_set && !confirm(&format!("Đã có giá trị cho `{env_name}`. Ghi đè?"))? {
        return Ok(());
    }
    let Some(key) = read_secret(&format!("Nhập API key cho `{env_name}`:"))? else {
        println!("Bỏ qua nhập key —Bean sẽ không gọi được LLM cho đến khi key được đặt.");
        return Ok(());
    };
    let key_string = SecretString::from(key);
    probe_llm(config, Some(&key_string)).await;
    store
        .set(registry, &env_name, key_string.clone())
        .with_context(|| format!("ghi secret `{env_name}` thất bại"))?;
    println!("Đã lưu `{env_name}` vào secrets.env (0600).");
    Ok(())
}

/// Bước 2: mật khẩu giao diện web.
fn setup_web_password(data_dir: &Path) -> Result<()> {
    println!("\n=== Bước 2/5: Mật khẩu giao diện web ===");
    let auth_path = data_dir.join("auth.toml");
    if auth_path.exists() && !confirm("Mật khẩu web đã đặt. Đổi mật khẩu?")? {
        println!("Bỏ qua bước 2.");
        return Ok(());
    }
    let password = match read_secret("Nhập mật khẩu web (ít nhất 12 ký tự):")? {
        Some(password) => password,
        None => {
            println!("Bỏ qua bước 2.");
            return Ok(());
        }
    };
    if password.len() < 12 {
        bail!("mật khẩu quá ngắn (ít nhất 12 ký tự)");
    }
    let confirmation = match read_secret("Nhập lại để xác nhận:")? {
        Some(confirmation) => confirmation,
        None => {
            println!("Bỏ qua bước 2.");
            return Ok(());
        }
    };
    if password != confirmation {
        bail!("giá trị xác nhận không khớp");
    }
    auth::set_password(data_dir, &password).context("ghi auth.toml thất bại")?;
    println!("Đã đặt mật khẩu web vào auth.toml (0600).");
    Ok(())
}

/// Bước 3: Telegram (tùy chọn).
fn setup_telegram(
    config: &mut Config,
    store: &SecretStore,
    registry: &SecretSlotRegistry,
    service: &SettingsService,
    audit: &AuditLog,
) -> Result<()> {
    println!("\n=== Bước 3/5: Telegram (tùy chọn) ===");
    let env_name = config.telegram.token_env.clone();
    if env_name.trim().is_empty() {
        println!("bean.toml chưa khai báo `telegram.token_env` — bỏ qua.");
        return Ok(());
    }
    if !confirm("Bật gửi tin chủ động qua Telegram?")? {
        println!("Bỏ qua bước 3.");
        return Ok(());
    }
    let already_set = slot_value(config, registry, &env_name).is_some();
    if already_set && !confirm(&format!("Đã có giá trị cho `{env_name}`. Ghi đè?"))? {
        return Ok(());
    }
    let Some(key) = read_secret("Nhập token Telegram (từ @BotFather):")? else {
        println!("Bỏ qua bước 3.");
        return Ok(());
    };
    let key_string = SecretString::from(key);
    store
        .set(registry, &env_name, key_string.clone())
        .with_context(|| format!("ghi secret `{env_name}` thất bại"))?;
    set_toml(service, audit, "telegram.enabled", "true")?;
    println!("Đã lưu `{env_name}` và bật kênh Telegram.");
    Ok(())
}

/// Bước 4: Tìm kiếm web (tùy chọn).
fn setup_search(
    config: &mut Config,
    store: &SecretStore,
    registry: &SecretSlotRegistry,
    service: &SettingsService,
    audit: &AuditLog,
) -> Result<()> {
    println!("\n=== Bước 4/5: Tìm kiếm web (tùy chọn) ===");
    let provider = config.tools.web_search.provider;
    let provider_label = match provider {
        WebSearchProvider::Tavily => "tavily",
        WebSearchProvider::Brave => "brave",
        WebSearchProvider::Searxng => "searxng",
    };
    let items: Vec<(String, String)> = [
        ("tavily", "Tavily"),
        ("brave", "Brave"),
        ("searxng", "SearXNG (tự host)"),
    ]
    .into_iter()
    .map(|(value, label)| {
        (
            value.to_string(),
            format!(
                "{label} {}",
                if provider_label == value {
                    "(đang dùng)"
                } else {
                    ""
                }
            ),
        )
    })
    .collect();
    let Some(idx) = choose("Chọn nhà cung cấp tìm kiếm:", &items)? else {
        println!("Bỏ qua bước 4.");
        return Ok(());
    };
    let new_provider = items[idx].0.as_str();
    let web_provider = match new_provider {
        "tavily" => WebSearchProvider::Tavily,
        "brave" => WebSearchProvider::Brave,
        "searxng" => WebSearchProvider::Searxng,
        other => bail!("provider không hợp lệ: {other}"),
    };
    set_toml(service, audit, "tools.web_search.provider", new_provider)?;
    config.tools.web_search.provider = web_provider;
    if let Some(base_url) = &config.tools.web_search.base_url {
        println!("Base URL hiện tại: {base_url}");
    }
    if let Some(base) = read_line("Nhập base URL mới (SearXNG, Enter để giữ):")?
        && !base.is_empty()
    {
        set_toml(service, audit, "tools.web_search.base_url", &base)?;
    }
    // Key (nếu provider cần).
    let env_name = config.tools.web_search.api_key_env.clone();
    if !env_name.trim().is_empty() {
        let already_set = slot_value(config, registry, &env_name).is_some();
        if already_set && !confirm(&format!("Đã có giá trị cho `{env_name}`. Ghi đè?"))? {
            return Ok(());
        }
        if let Some(key) = read_secret(&format!("Nhập API key cho `{env_name}`:"))? {
            store
                .set(registry, &env_name, SecretString::from(key.clone()))
                .with_context(|| format!("ghi secret `{env_name}` thất bại"))?;
            println!("Đã lưu `{env_name}` vào secrets.env (0600).");
        }
    }
    println!("Đã cấu hình tìm kiếm web.");
    Ok(())
}

/// Bước 5: Giọng nói (tùy chọn).
fn setup_voice(
    config: &mut Config,
    store: &SecretStore,
    registry: &SecretSlotRegistry,
    service: &SettingsService,
    audit: &AuditLog,
) -> Result<()> {
    println!("\n=== Bước 5/5: Giọng nói (tùy chọn) ===");
    let items = [
        (
            "local",
            "Local (whisper.cpp + Piper, không gửi audio ra ngoài máy)",
        ),
        ("api", "API (ElevenLabs; audio rời khỏi máy)"),
    ]
    .into_iter()
    .map(|(value, label)| (value.to_string(), label.to_string()))
    .collect::<Vec<_>>();
    let Some(idx) = choose("Chọn nhà cung cấp giọng nói:", &items)? else {
        println!("Bỏ qua bước 5.");
        return Ok(());
    };
    let new_provider = items[idx].0.as_str();
    set_toml(service, audit, "voice.provider", new_provider)?;
    config.voice.provider = bean_types::config::SpeechProviderKind::from_config_str(new_provider)
        .with_context(|| format!("provider không hợp lệ: {new_provider}"))?;
    if new_provider == "api" {
        let env_name = config.voice.api_key_env.clone();
        if env_name.trim().is_empty() {
            println!("bean.toml chưa khai báo `voice.api_key_env` — bỏ qua key.");
            return Ok(());
        }
        let already_set = slot_value(config, registry, &env_name).is_some();
        if already_set && !confirm(&format!("Đã có giá trị cho `{env_name}`. Ghi đè?"))? {
            return Ok(());
        }
        if let Some(key) = read_secret(&format!("Nhập API key ElevenLabs cho `{env_name}`:"))? {
            store
                .set(registry, &env_name, SecretString::from(key.clone()))
                .with_context(|| format!("ghi secret `{env_name}` thất bại"))?;
            println!("Đã lưu `{env_name}` vào secrets.env (0600).");
        }
    }
    println!("Đã cấu hình giọng nói.");
    Ok(())
}

/// Chạy wizard `bean setup` (idempotent: chạy lại được, mỗi bước có thể bỏ qua).
///
/// Mã thoát của tiến trình được quyết định bởi `bean doctor` cuối cùng (0/1/2).
pub async fn run_setup(config_path: Option<&Path>) -> Result<()> {
    let mut config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);
    let writable = config_path.is_some();
    if !writable {
        println!("Cảnh báo: không tìm thấy bean.toml — các bước ghi cấu hình sẽ bị bỏ qua.");
    }
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let registry = SecretSlotRegistry::from_config(&config);
    let service = SettingsService::new(&config, config_path);
    let audit = build_audit(&config)
        .context("mở audit log thất bại — thiết lập ghi cấu hình yêu cầu audit")?;

    println!("Bean setup — thiết lập tương tác ban đầu (mỗi bước có thể bỏ qua bằng Enter).\n");

    setup_provider(
        &mut config,
        &store,
        &registry,
        &service,
        &audit,
        config_path,
    )
    .await?;
    if writable {
        setup_web_password(&data_dir)?;
    } else {
        println!("\n=== Bước 2/5: Mật khẩu web (bỏ qua — không có bean.toml) ===");
    }
    if writable {
        setup_telegram(&mut config, &store, &registry, &service, &audit)?;
        setup_search(&mut config, &store, &registry, &service, &audit)?;
        setup_voice(&mut config, &store, &registry, &service, &audit)?;
    } else {
        println!("=== Bước 3-5/5: Telegram, tìm kiếm, giọng nói (bỏ qua — không có bean.toml) ===");
    }

    println!("\n=== Thiết lập xong. Chạy `bean doctor` để kiểm tra ===\n");
    let exit_code = crate::doctor::run(false, config_path).await?;
    std::process::exit(exit_code);
}

/// Chạy `bean model` — chọn provider/model tương tác, lưu vào `settings-state.json`
/// (cùng nơi với UI) và áp dụng khi khởi động lại Bean.
pub async fn run_model(config_path: Option<&Path>) -> Result<()> {
    let mut config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);
    let service = SettingsService::new(&config, config_path);
    let audit = build_audit(&config)
        .context("mở audit log thất bại — thiết lập ghi cấu hình yêu cầu audit")?;
    let active = config.llm.active_profile_name().to_string();
    println!(
        "Hiện tại: provider={} model={} profile={}",
        config.llm.provider.as_str(),
        config.llm.model,
        active
    );
    if !io::stdin().is_terminal() {
        println!(
            "Không có TTY — chỉ hiển thị trạng thái. Chạy `bean model` trong terminal để tương tác."
        );
        return Ok(());
    }
    loop {
        match choose(
            "Chọn thao tác:",
            &[
                ("model".to_string(), "Đổi model".to_string()),
                (
                    "provider".to_string(),
                    "Đổi nhà cung cấp (profile)".to_string(),
                ),
                ("quit".to_string(), "Thoát".to_string()),
            ],
        )? {
            Some(0) => {
                // Đổi model.
                let allowed = config.llm.effective_allowed_models();
                let model = if allowed.is_empty() {
                    match read_line("Nhập tên model:")? {
                        Some(model) if !model.is_empty() => model,
                        _ => continue,
                    }
                } else {
                    let items: Vec<(String, String)> = allowed
                        .iter()
                        .map(|model| (model.clone(), model.clone()))
                        .collect();
                    match choose("Chọn model:", &items)? {
                        Some(idx) => items[idx].1.clone(),
                        None => continue,
                    }
                };
                set_toml(&service, &audit, "llm.model", &model)?;
                save_settings_state(&data_dir, &active, &model)?;
                config.llm.model = model.clone();
                println!("Đã lưu model `{model}` (áp dụng khi khởi động lại Bean).");
            }
            Some(1) => {
                // Đổi nhà cung cấp.
                let profiles = config.llm.profiles();
                let items: Vec<(String, String)> = profiles
                    .iter()
                    .map(|profile| (profile.name.clone(), profile.model.clone()))
                    .collect();
                let Some(idx) = choose("Chọn profile:", &items)? else {
                    continue;
                };
                let profile = &profiles[idx];
                for (key, value) in [
                    (
                        "llm.allowed_models",
                        serde_json::to_string(&profile.allowed_models).with_context(|| {
                            format!("serialize allowed_models cho profile `{}`", profile.name)
                        })?,
                    ),
                    ("llm.base_url", profile.base_url.clone().unwrap_or_default()),
                    ("llm.max_tokens", profile.max_tokens.to_string()),
                    ("llm.api_key_env", profile.api_key_env.clone()),
                    ("llm.provider", profile.provider.as_str().to_string()),
                    ("llm.model", profile.model.clone()),
                ] {
                    set_toml(&service, &audit, key, &value)?;
                }
                config.llm.apply_profile(profile);
                save_settings_state(&data_dir, &profile.name, &profile.model)?;
                println!(
                    "Đã chuyển sang profile `{}` và lưu settings-state.json.",
                    profile.name
                );
            }
            Some(_) | None => break,
        }
    }
    Ok(())
}
