//! `bean doctor` — kiểm tra sức khoẻ cấu hình (M29 mục 7).
//!
//! Kiểm tra offline: quyền file (`data.dir` 0700, secret/state 0600), key sai chỗ
//! (`bean.toml`/`voice-state.json`/`~/.config/bean/env`), slot bị env che (so giá trị
//! trong bộ nhớ, không in), biến proxy (K29), tiến trình Bean/port web (K27),
//! `llm-state.json` ghi đè `bean.toml` (22.24), sandbox Docker, Chrome bản snap.
//!
//! `--online` (opt-in): request tối thiểu tới LLM, ElevenLabs (voice api), Telegram
//! getMe và web search. Lỗi mạng/token luôn redact URL — Telegram có token trong URL,
//! in nguyên lỗi là lộ token.
//!
//! Không bao giờ in giá trị secret. Mã thoát: `0` ok, `1` lỗi, `2` cảnh báo — để
//! script dùng được (`bean doctor >/dev/null || ...`).

use std::net::TcpListener;
use std::path::{Path, PathBuf};

use anyhow::Context;
use bean_settings::{ProbeLevel, ProbeOutcome, SecretSlotRegistry, SecretStore, env_first_lookup};
use bean_types::config::{
    Config, DEFAULT_CONFIG_FILE, SETTINGS_STATE_FILE, SettingsState, SpeechProviderKind,
};

use crate::chat::expand_tilde;

/// Biến môi trường proxy cần cảnh báo (K29).
const PROXY_VARS: &[&str] = &[
    "ALL_PROXY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "all_proxy",
    "https_proxy",
    "http_proxy",
];

/// Tên binary Chrome/Chromium để tự phát hiện khi `[browser].executable_path` rỗng —
/// cùng thứ tự với `chromiumoxide::detection` mà bean-browser dùng (D26.2).
const CHROME_CANDIDATES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
];

/// Mức của một lần phát hiện: ảnh hưởng tới mã thoát của `doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Ok,
    Warn,
    Error,
}

/// Một dòng kết quả kiểm tra — kèm gợi ý xử lý khi có vấn đề.
#[derive(Debug)]
struct Finding {
    severity: Severity,
    message: String,
    hint: Option<String>,
}

impl Finding {
    fn ok(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Ok,
            message: message.into(),
            hint: None,
        }
    }

    fn warn(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warn,
            message: message.into(),
            hint: None,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            hint: None,
        }
    }

    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Probe dùng chung của `bean-settings` (M29) đổi thành Finding của CLI — message
/// và hint giữ nguyên văn, chỉ ánh xạ mức.
impl From<ProbeOutcome> for Finding {
    fn from(outcome: ProbeOutcome) -> Self {
        let severity = match outcome.level {
            ProbeLevel::Ok => Severity::Ok,
            ProbeLevel::Warn => Severity::Warn,
            ProbeLevel::Error => Severity::Error,
        };
        Self {
            severity,
            message: outcome.message,
            hint: outcome.hint,
        }
    }
}

/// Mã thoát theo mức xấu nhất: lỗi > cảnh báo > ok.
fn exit_code(findings: &[Finding]) -> i32 {
    if findings
        .iter()
        .any(|finding| finding.severity == Severity::Error)
    {
        1
    } else if findings
        .iter()
        .any(|finding| finding.severity == Severity::Warn)
    {
        2
    } else {
        0
    }
}

/// Chạy lệnh `doctor`.
///
/// # Errors
/// Chỉ khi không nạp được cấu hình; bản thân các check không bao giờ `Err` — vấn đề
/// được báo qua [`Finding`] để in đủ danh sách thay vì dừng ở lỗi đầu tiên.
pub async fn run(online: bool, config_path: Option<&Path>) -> anyhow::Result<i32> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let data_dir = expand_tilde(&config.data.dir);

    let mut findings = Vec::new();
    check_permissions(config_path, &data_dir, &mut findings);
    check_misplaced_keys(&config, &data_dir, &mut findings);
    check_shadowed_slots(&config, &data_dir, &mut findings);
    check_proxy_env(&mut findings);
    check_processes_and_port(&config, &mut findings);
    check_llm_state_override(&config, &data_dir, &mut findings);
    check_voice_setup(&config, &data_dir, &mut findings);
    check_sandbox(&config, &mut findings);
    check_chrome(&config, &mut findings);
    if online {
        online_checks(&config, &data_dir, &mut findings).await;
    }

    println!("bean doctor — kiểm tra sức khoẻ cấu hình (M29 mục 7)");
    match config_path {
        Some(path) => println!("cấu hình: {}", path.display()),
        None => println!("cấu hình: mặc định ({DEFAULT_CONFIG_FILE})"),
    }
    if online {
        println!("chế độ: --online (request thật tới provider — một call nhỏ mỗi target)");
    }
    println!();
    print_findings(&findings);
    Ok(exit_code(&findings))
}

fn print_findings(findings: &[Finding]) {
    let mut errors = 0usize;
    let mut warns = 0usize;
    for finding in findings {
        match finding.severity {
            Severity::Ok => println!("  ✓  {}", finding.message),
            Severity::Warn => {
                warns += 1;
                println!("  ⚠  CẢNH BÁO: {}", finding.message);
            }
            Severity::Error => {
                errors += 1;
                println!("  ✗  LỖI: {}", finding.message);
            }
        }
        if let Some(hint) = &finding.hint {
            println!("     ↳ {hint}");
        }
    }
    let oks = findings.len() - errors - warns;
    println!();
    println!("Kết quả: {oks} ok · {warns} cảnh báo · {errors} lỗi");
}

// ---------------------------------------------------------------------------
// Quyền file
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

/// Kiểm một file/thư mục: rộng hơn mức mong đợi là cảnh báo (không phải lỗi —
/// người dùng có thể chủ động để rộng, nhưng phải biết).
#[cfg(unix)]
fn check_file_mode(path: &Path, expected: u32, label: &str, findings: &mut Vec<Finding>) {
    if !path.exists() {
        findings.push(Finding::ok(format!(
            "{label}: chưa có (bình thường nếu chưa dùng lần nào)"
        )));
        return;
    }
    match file_mode(path) {
        Some(mode) if mode & !expected == 0 => {
            findings.push(Finding::ok(format!("{label}: quyền {mode:o} đúng")));
        }
        Some(mode) => findings.push(
            Finding::warn(format!("{label}: quyền {mode:o} rộng hơn {expected:o}"))
                .with_hint(format!("chmod {expected:o} {}", path.display())),
        ),
        None => findings.push(Finding::warn(format!("{label}: không đọc được quyền"))),
    }
}

fn check_permissions(config_path: Option<&Path>, data_dir: &Path, findings: &mut Vec<Finding>) {
    #[cfg(not(unix))]
    {
        let _ = (config_path, data_dir);
        findings.push(Finding::ok("bỏ qua kiểm tra quyền (nền tảng không POSIX)"));
    }
    #[cfg(unix)]
    {
        if !data_dir.exists() {
            findings.push(
                Finding::warn(format!("data.dir `{}` chưa tồn tại", data_dir.display()))
                    .with_hint("sẽ được tạo khi chạy `bean serve` hoặc `bean auth set-password`"),
            );
        } else {
            check_file_mode(data_dir, 0o700, "data.dir", findings);
        }
        check_file_mode(
            &data_dir.join("secrets.env"),
            0o600,
            "secrets.env",
            findings,
        );
        check_file_mode(&data_dir.join("auth.toml"), 0o600, "auth.toml", findings);
        check_file_mode(
            &data_dir.join(SETTINGS_STATE_FILE),
            0o600,
            "settings-state.json",
            findings,
        );
        let toml_path =
            config_path.map_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE), Path::to_path_buf);
        check_file_mode(&toml_path, 0o600, "bean.toml", findings);
    }
}

// ---------------------------------------------------------------------------
// Key sai chỗ
// ---------------------------------------------------------------------------

fn non_empty(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn check_misplaced_keys(config: &Config, _data_dir: &Path, findings: &mut Vec<Finding>) {
    const MIGRATE_HINT: &str = "bean config migrate-secrets --apply";
    if non_empty(config.llm.api_key.as_deref()) {
        findings.push(
            Finding::warn(
                "[llm].api_key còn plaintext trong bean.toml — deprecated (M29), key nên nằm trong secrets.env",
            )
            .with_hint(MIGRATE_HINT),
        );
    }
    for profile in &config.llm.providers {
        if non_empty(profile.api_key.as_deref()) {
            findings.push(
                Finding::warn(format!(
                    "[[llm.providers]] `{}` còn api_key plaintext trong bean.toml — deprecated (M29)",
                    profile.name
                ))
                .with_hint(MIGRATE_HINT),
            );
        }
    }
    let env_file = std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".config/bean/env"));
    if let Some(env_file) = env_file
        && env_file.is_file()
    {
        findings.push(
            Finding::warn(format!(
                "`{}` còn tồn tại — beanctl (M29) không nạp file này nữa",
                env_file.display()
            ))
            .with_hint(
                "`bean config migrate-secrets --from-env-file` để chuyển key còn lại rồi xoá file",
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Slot bị env che
// ---------------------------------------------------------------------------

fn check_shadowed_slots(config: &Config, data_dir: &Path, findings: &mut Vec<Finding>) {
    let registry = SecretSlotRegistry::from_config(config);
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let Ok(snapshot) = store.snapshot() else {
        findings.push(Finding::warn(
            "không đọc được secrets.env — secret chỉ resolve từ biến môi trường",
        ));
        return;
    };
    // So sánh giá trị trực tiếp trong bộ nhớ — không in, không hash ra ngoài.
    let mut shadowed: Vec<&str> = Vec::new();
    for slot in registry.slots() {
        let from_env = std::env::var(&slot.name)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let Some(from_env) = from_env else { continue };
        let Some(from_file) = snapshot.lookup_str(&slot.name) else {
            continue;
        };
        if from_env != from_file {
            shadowed.push(&slot.name);
        }
    }
    if shadowed.is_empty() {
        findings.push(Finding::ok("không có slot nào bị env che"));
    } else {
        for name in shadowed {
            findings.push(
                Finding::warn(format!(
                    "slot `{name}` bị env che: biến môi trường THẮNG (D19.3) nhưng secrets.env có giá trị KHÁC — đổi env mà không có tác dụng sẽ rất khó đoán (K28)"
                ))
                .with_hint(format!(
                    "unset biến môi trường {name} (kiểm ~/.bashrc, ~/.profile, unit systemd) hoặc `bean secret set {name}` để đồng bộ"
                )),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Môi trường: proxy, tiến trình, port
// ---------------------------------------------------------------------------

fn check_proxy_env(findings: &mut Vec<Finding>) {
    let set: Vec<&str> = PROXY_VARS
        .iter()
        .copied()
        .filter(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
        .collect();
    if set.is_empty() {
        findings.push(Finding::ok("không có biến proxy trong môi trường"));
    } else {
        findings.push(
            Finding::warn(format!(
                "biến proxy đang đặt: {} — proxy hỏng làm MỌI call ra ngoài (LLM/voice/search) trả lỗi, khó đoán (K29)",
                set.join(", ")
            ))
            .with_hint(
                "gỡ khỏi môi trường tiến trình Bean kế thừa (đừng dựng env sạch bằng `env -i` — mất DISPLAY/XAUTHORITY)",
            ),
        );
    }
}

/// Đếm tiến trình `bean` khác đang chạy (đọc `/proc/*/cmdline`, loại trừ chính
/// doctor). Nền tảng khác Linux: luôn 0 (check này chỉ mang tính gợi ý).
#[cfg(target_os = "linux")]
fn count_other_bean_processes() -> usize {
    let self_pid = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == self_pid {
            continue;
        }
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        // argv cách nhau bằng NUL; token đầu là đường dẫn binary.
        let Some(first) = raw.split(|byte| *byte == 0).find(|part| !part.is_empty()) else {
            continue;
        };
        let Ok(exe) = std::str::from_utf8(first) else {
            continue;
        };
        if Path::new(exe).file_name().and_then(|os| os.to_str()) == Some("bean") {
            count += 1;
        }
    }
    count
}

#[cfg(not(target_os = "linux"))]
fn count_other_bean_processes() -> usize {
    0
}

fn check_processes_and_port(config: &Config, findings: &mut Vec<Finding>) {
    let others = count_other_bean_processes();
    match others {
        0 => findings.push(Finding::ok(
            "không thấy tiến trình Bean nào khác đang chạy",
        )),
        1 => findings.push(Finding::ok(
            "1 tiến trình Bean khác đang chạy (serve?) — bình thường",
        )),
        n => findings.push(
            Finding::warn(format!(
                "{n} tiến trình Bean khác đang chạy — tranh data.dir/SQLite và scheduler, task chạy đôi (K27)"
            ))
            .with_hint("scripts/beanctl status; scripts/beanctl restart"),
        ),
    }
    if !config.web.enabled {
        return;
    }
    match TcpListener::bind(config.web.bind) {
        Ok(listener) => {
            drop(listener);
            if others == 0 {
                findings.push(
                    Finding::warn(format!(
                        "web.bind {} còn trống và không có tiến trình Bean nào — Bean KHÔNG đang chạy",
                        config.web.bind
                    ))
                    .with_hint("scripts/beanctl start"),
                );
            } else {
                findings.push(Finding::ok(format!(
                    "web.bind {} rảnh — instance Bean đang chạy có bind khác?",
                    config.web.bind
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if others == 0 {
                findings.push(
                    Finding::warn(format!(
                        "web.bind {} đang bị chiếm nhưng không thấy tiến trình Bean — tiến trình khác giữ port, `serve` sẽ chết với `Address already in use`",
                        config.web.bind
                    ))
                    .with_hint("lsof -i :<port> rồi kill đúng PID (đừng pkill -f \"bean serve\" — không khớp cmdline thật)"),
                );
            } else {
                findings.push(Finding::ok(format!(
                    "web.bind {} do instance Bean khác giữ — bình thường",
                    config.web.bind
                )));
            }
        }
        Err(error) => findings.push(Finding::warn(format!(
            "không thử được bind {}: {error}",
            config.web.bind
        ))),
    }
}

// ---------------------------------------------------------------------------
// llm-state đè bean.toml
// ---------------------------------------------------------------------------

fn check_llm_state_override(config: &Config, data_dir: &Path, findings: &mut Vec<Finding>) {
    match SettingsState::load(&data_dir.join(SETTINGS_STATE_FILE)) {
        Some(state) => {
            if state.llm_profile == config.llm.active_profile_name()
                && state.llm_model == config.llm.model
            {
                findings.push(Finding::ok(format!(
                    "settings-state.json khớp bean.toml (profile `{}`, model `{}`)",
                    state.llm_profile, state.llm_model
                )));
            } else {
                findings.push(
                    Finding::ok(format!(
                        "settings-state.json đang GHI ĐÈ bean.toml: profile `{}` model `{}` — bean.toml khai model `{}`; file state THẮNG lúc khởi động (22.24)",
                        state.llm_profile, state.llm_model, config.llm.model
                    ))
                    .with_hint("muốn về theo bean.toml: `bean config unset llm.model`"),
                );
            }
        }
        None => findings.push(Finding::ok(
            "settings-state.json không có — model theo bean.toml",
        )),
    }
}

// ---------------------------------------------------------------------------
// Voice (offline)
// ---------------------------------------------------------------------------

fn which_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn check_local_command(label: &str, command: &str, findings: &mut Vec<Finding>) {
    if command.trim().is_empty() {
        findings.push(
            Finding::warn(format!("voice local: {label} rỗng"))
                .with_hint("sửa [voice] trong bean.toml hoặc `bean config set`"),
        );
        return;
    }
    let resolved = if command.contains('/') {
        Some(PathBuf::from(command))
    } else {
        which_in_path(command)
    };
    match resolved {
        Some(path) if path.is_file() => findings.push(Finding::ok(format!(
            "voice local: {label} tìm thấy ({})",
            path.display()
        ))),
        Some(path) => findings.push(Finding::warn(format!(
            "voice local: {label} trỏ tới `{}` nhưng không phải file",
            path.display()
        ))),
        None => findings.push(
            Finding::warn(format!(
                "voice local: {label} `{command}` không thấy trên PATH"
            ))
            .with_hint("cài whisper.cpp/piper hoặc sửa [voice] trong bean.toml"),
        ),
    }
}

fn check_local_model(label: &str, model: &str, findings: &mut Vec<Finding>) {
    if model.trim().is_empty() {
        findings.push(
            Finding::warn(format!(
                "voice local: {label} rỗng — chế độ local không chạy được"
            ))
            .with_hint("đặt đường dẫn model trong [voice] (bean.toml)"),
        );
        return;
    }
    let path = expand_tilde(Path::new(model));
    if path.is_file() {
        findings.push(Finding::ok(format!(
            "voice local: {label} có sẵn ({})",
            path.display()
        )));
    } else {
        findings.push(
            Finding::warn(format!(
                "voice local: {label} không thấy file `{}`",
                path.display()
            ))
            .with_hint("tải model whisper.cpp/piper về đúng đường dẫn trong bean.toml"),
        );
    }
}

fn check_voice_setup(config: &Config, data_dir: &Path, findings: &mut Vec<Finding>) {
    if !config.voice.enabled {
        return;
    }
    match config.voice.provider {
        SpeechProviderKind::Local => {
            check_local_command("whisper (STT)", &config.voice.whisper_command, findings);
            check_local_model("whisper model", &config.voice.whisper_model, findings);
            check_local_command("piper (TTS)", &config.voice.piper_command, findings);
            check_local_model("piper model", &config.voice.piper_model, findings);
        }
        SpeechProviderKind::Api => {
            // Key resolve được hay chưa — check offline; request thật ở --online.
            let store = SecretStore::new(data_dir.join("secrets.env"));
            let snapshot = store.snapshot().ok();
            let lookup = env_first_lookup(snapshot.as_ref());
            let resolved = config.resolve_voice_api_key_with(&lookup).ok().flatten();
            if resolved.is_none() {
                findings.push(
                    Finding::warn(format!(
                        "voice api: chưa resolve được key từ `{}` (env/secrets.env)",
                        config.voice.api_key_env
                    ))
                    .with_hint(format!("`bean secret set {}`", config.voice.api_key_env)),
                );
            } else {
                findings.push(Finding::ok(
                    "voice api: key resolve được (env hoặc secrets.env)",
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sandbox Docker + Chrome
// ---------------------------------------------------------------------------

fn check_sandbox(config: &Config, findings: &mut Vec<Finding>) {
    if config.security.sandbox.mode.as_str() != "docker" {
        findings.push(
            Finding::warn(
                "sandbox mode = host — mọi lệnh shell là Dangerous và chạy trực tiếp trên máy",
            )
            .with_hint("chỉ bật khi hiểu rõ; ưu tiên docker"),
        );
        return;
    }
    match std::process::Command::new("id").arg("-nG").output() {
        Ok(output) if output.status.success() => {
            let groups = String::from_utf8_lossy(&output.stdout);
            if groups.split_whitespace().any(|group| group == "docker") {
                findings.push(Finding::ok("user thuộc nhóm docker — sandbox dùng được"));
            } else {
                findings.push(
                    Finding::error("user KHÔNG thuộc nhóm docker — mọi run_shell sẽ hỏng")
                        .with_hint(
                            "sudo usermod -aG docker $USER rồi đăng nhập lại (nhóm docker ≈ root)",
                        ),
                );
            }
        }
        _ => findings.push(Finding::warn(
            "không chạy được `id -nG` — bỏ qua kiểm tra nhóm docker",
        )),
    }
    match std::process::Command::new("docker")
        .args(["image", "inspect", &config.security.sandbox.image])
        .output()
    {
        Ok(output) if output.status.success() => findings.push(Finding::ok(format!(
            "image sandbox `{}` có sẵn",
            config.security.sandbox.image
        ))),
        Ok(_) => findings.push(
            Finding::error(format!(
                "image sandbox `{}` chưa có — mọi run_shell sẽ hỏng",
                config.security.sandbox.image
            ))
            .with_hint(format!(
                "docker build -f Dockerfile.sandbox -t {} . (hoặc chỉnh [security.sandbox.image])",
                config.security.sandbox.image
            )),
        ),
        Err(_) => findings.push(
            Finding::error("không chạy được lệnh `docker` — sandbox mode=docker sẽ hỏng mọi run_shell")
                .with_hint(
                    "cài docker, hoặc đổi security.sandbox.mode (khoá tầng B: `bean config set security.sandbox.mode host`)",
                ),
        ),
    }
}

fn find_chrome(configured: &str) -> Option<PathBuf> {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        return path.is_file().then_some(path);
    }
    if let Ok(from_env) = std::env::var("CHROME") {
        let path = PathBuf::from(from_env.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    for name in CHROME_CANDIDATES {
        if let Some(path) = which_in_path(name) {
            return Some(path);
        }
    }
    let fallback = PathBuf::from("/opt/google/chrome/chrome");
    fallback.is_file().then_some(fallback)
}

fn check_chrome(config: &Config, findings: &mut Vec<Finding>) {
    if !config.browser.enabled {
        return;
    }
    match find_chrome(&config.browser.executable_path) {
        Some(path) => {
            let resolved = std::fs::canonicalize(&path).unwrap_or(path);
            let display = resolved.display().to_string();
            if display.contains("/snap/") {
                findings.push(
                    Finding::warn(format!(
                        "Chrome là bản snap ({display}) — hay lỗi CDP/sandbox trên Ubuntu"
                    ))
                    .with_hint("dùng bản .deb (google-chrome-stable) thay vì snap"),
                );
            } else {
                findings.push(Finding::ok(format!("Chrome tìm thấy: {display}")));
            }
        }
        None => findings.push(
            Finding::warn("tool browser bật nhưng không tìm thấy Chrome/Chromium")
                .with_hint("đặt [browser].executable_path hoặc cài google-chrome"),
        ),
    }
}

// ---------------------------------------------------------------------------
// Checks online (--online) — phần HTTP nằm ở `bean-settings::probe` (M29, một
// nguồn sự thật cho cả REST `settings/test`); hàm dưới chỉ resolve secret rồi
// gọi probe, không tự dựng request.
// ---------------------------------------------------------------------------

async fn online_checks(config: &Config, data_dir: &Path, findings: &mut Vec<Finding>) {
    findings.push(llm_ping(config, data_dir).await);
    if config.voice.enabled && matches!(config.voice.provider, SpeechProviderKind::Api) {
        findings.push(voice_ping(config, data_dir).await);
    }
    if config.telegram.enabled {
        findings.push(telegram_ping(config, data_dir).await);
    }
    if let Some(finding) = search_ping(config, data_dir).await {
        findings.push(finding);
    }
}

async fn llm_ping(config: &Config, data_dir: &Path) -> Finding {
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let snapshot = store.snapshot().ok();
    let lookup = env_first_lookup(snapshot.as_ref());
    let secrets = match config.resolve_secrets_with(lookup) {
        Ok(secrets) => secrets,
        Err(error) => {
            return Finding::error(format!("không resolve được secret LLM: {error}"))
                .with_hint(format!("`bean secret set {}`", config.llm.api_key_env));
        }
    };
    let provider = match bean_llm::build_provider(&config.llm, secrets.llm_api_key) {
        Ok(provider) => provider,
        Err(error) => return Finding::error(format!("không dựng được provider LLM: {error}")),
    };
    bean_settings::probe_llm(
        provider.as_ref(),
        config.llm.provider.as_str(),
        &config.llm.model,
    )
    .await
    .into()
}

async fn voice_ping(config: &Config, data_dir: &Path) -> Finding {
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let snapshot = store.snapshot().ok();
    let lookup = env_first_lookup(snapshot.as_ref());
    let key = match config.resolve_voice_api_key_with(&lookup) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return Finding::error(format!(
                "voice api: chưa có key ({})",
                config.voice.api_key_env
            ))
            .with_hint(format!("`bean secret set {}`", config.voice.api_key_env));
        }
        Err(error) => {
            return Finding::error(format!("voice api: đọc key thất bại: {error}"))
                .with_hint(format!("`bean secret set {}`", config.voice.api_key_env));
        }
    };
    bean_settings::probe_voice_api(&config.voice.api_base_url, &key)
        .await
        .into()
}

async fn telegram_ping(config: &Config, data_dir: &Path) -> Finding {
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let snapshot = store.snapshot().ok();
    let lookup = env_first_lookup(snapshot.as_ref());
    let token = match config.resolve_telegram_token_with(&lookup) {
        Ok(Some(token)) => token,
        Ok(None) => {
            return Finding::error(format!(
                "telegram: chưa có token ({})",
                config.telegram.token_env
            ))
            .with_hint(format!("`bean secret set {}`", config.telegram.token_env));
        }
        Err(error) => {
            return Finding::error(format!("telegram: đọc token thất bại: {error}"))
                .with_hint(format!("`bean secret set {}`", config.telegram.token_env));
        }
    };
    bean_settings::probe_telegram(&token).await.into()
}

async fn search_ping(config: &Config, data_dir: &Path) -> Option<Finding> {
    if !config.tools.enabled.iter().any(|group| group == "web") {
        return None;
    }
    let provider = config.tools.web_search.provider;
    let store = SecretStore::new(data_dir.join("secrets.env"));
    let snapshot = store.snapshot().ok();
    let lookup = env_first_lookup(snapshot.as_ref());
    let key = if provider.requires_api_key() {
        match config.resolve_web_search_api_key_with(&lookup) {
            Some(key) => Some(key),
            None => {
                return Some(
                    Finding::error(format!(
                        "web_search: chưa có key ({})",
                        config.tools.web_search.api_key_env
                    ))
                    .with_hint(format!(
                        "`bean secret set {}`",
                        config.tools.web_search.api_key_env
                    )),
                );
            }
        }
    } else {
        None
    };
    Some(
        bean_settings::probe_search(
            provider,
            config.tools.web_search.base_url.as_deref(),
            key.as_ref(),
        )
        .await
        .into(),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn exit_code_reflects_worst_severity() {
        assert_eq!(exit_code(&[]), 0);
        assert_eq!(exit_code(&[Finding::ok("x")]), 0);
        assert_eq!(exit_code(&[Finding::ok("x"), Finding::warn("y")]), 2);
        assert_eq!(exit_code(&[Finding::warn("y"), Finding::error("z")]), 1);
    }

    #[test]
    fn misplaced_keys_are_flagged() {
        let mut config = Config::default();
        config.llm.api_key = Some("key".to_string());
        let dir = tempfile::tempdir().unwrap();

        let mut findings = Vec::new();
        check_misplaced_keys(&config, dir.path(), &mut findings);
        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("[llm].api_key")),
            "phải gắn cờ api_key trong bean.toml: {:?}",
            findings
        );

        // settings-state.json không còn lưu api_key — check này chỉ còn ý nghĩa legacy.
        // Giữ test để đảm bảo không panics khi file không tồn tại.
        let mut findings = Vec::new();
        check_misplaced_keys(&config, dir.path(), &mut findings);
    }

    #[test]
    fn profile_key_is_flagged_too() {
        let profile = bean_types::config::LlmProfile {
            name: "second".to_string(),
            provider: bean_types::config::LlmProviderKind::OpenAiCompat,
            model: "test-model".to_string(),
            allowed_models: Vec::new(),
            api_key: Some("key".to_string()),
            api_key_env: String::new(),
            base_url: None,
            max_tokens: 4096,
        };
        let mut config = Config::default();
        config.llm.providers.push(profile);

        let dir = tempfile::tempdir().unwrap();
        let mut findings = Vec::new();
        check_misplaced_keys(&config, dir.path(), &mut findings);
        assert!(
            findings
                .iter()
                .any(|finding| finding.message.contains("`second`")),
            "phải gắn cờ api_key của profile: {:?}",
            findings
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_mode_check_flags_wide_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.env");
        std::fs::write(&path, "A=1\n").unwrap();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut findings = Vec::new();
        check_file_mode(&path, 0o600, "secrets.env", &mut findings);
        assert!(matches!(findings[0].severity, Severity::Warn));
        assert!(
            findings[0]
                .hint
                .as_deref()
                .is_some_and(|hint| hint.contains("chmod 600"))
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut findings = Vec::new();
        check_file_mode(&path, 0o600, "secrets.env", &mut findings);
        assert!(matches!(findings[0].severity, Severity::Ok));
    }
}
