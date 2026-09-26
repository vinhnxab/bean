//! `BeanAgent chat` — REPL tối giản.
//!
//! * ~~M1~~: `FakeProvider` (echo / kịch bản `--fake-llm`); chưa có agent loop.
//! * **M2**: provider thật theo `[llm]` qua `beanagent_llm::build_provider`
//!   (một lượt, chưa có tool). `--fake-llm` vẫn ghi đè lên provider thật — hữu ích
//!   cho demo và test end-to-end không cần mạng.
//! * **M3**: thêm agent loop + tool; **M8**: chuyển sang đi qua `Router` như một `Channel`
//!   (`channel = "cli"`, `chat_id = "local"`), Ctrl-C trở thành `cancel` của run đang chạy.
//! * **M5**: lịch sử hội thoại lưu bền vững vào SQLite + FTS5 (`data.dir/beanagent.db`).
//!   `/new` lưu trữ phiên cũ rồi mở phiên mới; tool `memory_save`/`memory_search` được
//!   đăng ký khi nhóm tool `memory` bật.
//!
//! Hai chế độ vào:
//! * có TTY → `rustyline` (lịch sử, sửa dòng);
//! * không có TTY (pipe/file) → đọc từng dòng từ stdin, phục vụ `echo "…" | BeanAgent chat`,
//!   test CLI và script end-to-end ở M16.

use std::collections::VecDeque;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use beanagent_billing::{BillingClient, billing_read_cost};
use beanagent_core::{
    Channel, Decision, Incoming, Router, RouterDeps, SqliteStore, SystemClock, memory_tools,
    schedule_tools,
};
use beanagent_llm::{FakeProvider, LlmProvider};
use beanagent_marketing::{PublishClient, marketing_draft, marketing_publish};
use beanagent_scan::{ScanScope, ScannerCmd, security_scan};
use beanagent_security::{
    AuditLog, CapWorkspace, SafeHttpClient, Sandbox, run_shell_for_projects, web_fetch, web_search,
};
use beanagent_skills::{SkillCatalog, skill_tools};
use beanagent_tools::{ToolRegistry, mcp::McpRuntime};
use beanagent_types::{Config, Outbound, RunEvent, RunId};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use crate::cli::ChatArgs;

const PROMPT: &str = "bạn> ";

/// Mở audit log cho Router; lỗi chỉ cảnh báo, không chặn chat.
pub(crate) fn build_audit(config: &Config) -> Option<Arc<AuditLog>> {
    let audit_dir = expand_tilde(&config.data.dir).join("audit");
    match AuditLog::open(&audit_dir) {
        Ok(log) => Some(Arc::new(log)),
        Err(error) => {
            eprintln!(
                "cảnh báo: không mở được audit log ở {}: {error}",
                audit_dir.display()
            );
            None
        }
    }
}

/// Mở rộng `~` trong đường dẫn cấu hình bằng biến môi trường `HOME`.
pub(crate) fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

/// Đường dẫn đến SQLite database.
pub(crate) fn store_path(config: &Config) -> PathBuf {
    expand_tilde(&config.data.dir).join("beanagent.db")
}

/// Chạy REPL.
///
/// # Errors
/// Lỗi khi nạp cấu hình/kịch bản/secret, hoặc khi không mở được terminal.
pub async fn run(args: &ChatArgs, config_path: Option<&Path>) -> Result<()> {
    let mut config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    if let Some(workspace) = &args.workspace {
        config.agent.workspace = workspace.clone();
    }
    let (provider, web_search_api_key) = build_provider(args, &config)?;
    let store = Arc::new(
        SqliteStore::open(&store_path(&config))
            .map_err(|error| anyhow::anyhow!("không mở được store: {error}"))?,
    );
    let user_skills_root = expand_tilde(&config.data.dir).join("skills");
    let project_skills_root = PathBuf::from("skills");
    let skills = SkillCatalog::load_with_paths(
        &[project_skills_root.clone(), user_skills_root.clone()],
        user_skills_root,
        project_skills_root.join("_drafts"),
    );
    let skills_index = if config.tools.enabled.iter().any(|group| group == "skills") {
        skills.index()
    } else {
        String::new()
    };
    let built = build_registry(&config, store.clone(), skills.clone(), web_search_api_key).await?;
    let mcp = built.mcp;
    let registry = Arc::new(built.registry);
    let audit = build_audit(&config);
    let workspace = config.agent.workspace.display().to_string();
    let router = Arc::new(Router::new(RouterDeps {
        config,
        store,
        registry,
        llm: provider.clone(),
        audit,
        skills_index,
        skills: Some(skills),
    }));
    router
        .start_outbox_worker()
        .context("không khởi động được worker outbox")?;

    let piped = if std::io::stdin().is_terminal() {
        None
    } else {
        Some(read_piped_lines().await?)
    };
    let channel = Arc::new(CliChannel::new(piped));
    if let Err(error) = router.register_channel(channel.clone()) {
        mcp.close().await;
        return Err(error).context("đăng ký CLI channel thất bại");
    }

    println!(
        "BeanAgent chat — provider: {}. Gõ /exit hoặc Ctrl-D để thoát.",
        provider.name()
    );
    println!("workspace: {workspace}");
    let run_result = channel.run(router.clone(), CancellationToken::new()).await;
    router.shutdown();
    mcp.close().await;
    run_result.context("CLI channel dừng lỗi")
}

/// Chọn provider: kịch bản `--fake-llm` nếu có, ngược lại provider thật theo `[llm]`.
///
/// Lỗi trả về luôn nêu rõ **nguyên nhân cấu hình** (thiếu biến môi trường, sai provider…)
/// chứ không lộ giá trị secret.
pub(crate) fn build_provider(
    args: &ChatArgs,
    config: &Config,
) -> Result<(Arc<dyn LlmProvider>, Option<SecretString>)> {
    if let Some(path) = args.fake_llm.as_deref() {
        let provider =
            FakeProvider::from_json_path(path).context("nạp kịch bản --fake-llm thất bại")?;
        return Ok((Arc::new(provider), config.resolve_web_search_api_key()));
    }
    let secrets = config
        .resolve_secrets()
        .context("đọc secret từ biến môi trường thất bại")?;
    let web_search_api_key = secrets.web_search_api_key.clone();
    let provider = beanagent_llm::build_provider(&config.llm, secrets.llm_api_key)
        .context("dựng provider LLM thất bại (kiểm tra [llm] trong BeanAgent.toml)")?;
    Ok((provider, web_search_api_key))
}

/// Registry tool và các kết nối MCP cần shutdown cùng tiến trình.
pub(crate) struct BuiltRegistry {
    /// Registry dùng bởi Router.
    pub(crate) registry: ToolRegistry,
    /// Connection MCP để đóng tường minh khi `chat`/`serve` kết thúc.
    pub(crate) mcp: McpRuntime,
}

/// Xây registry tool từ cấu hình — tự tạo `agent.workspace` nếu chưa tồn tại (mục 4).
/// Path jail bằng `CapWorkspace`; MCP stdio nạp ở M14 và lỗi từng server không chặn startup.
pub(crate) async fn build_registry(
    config: &Config,
    store: Arc<SqliteStore>,
    skills: SkillCatalog,
    web_search_api_key: Option<SecretString>,
) -> Result<BuiltRegistry> {
    std::fs::create_dir_all(&config.agent.workspace).with_context(|| {
        format!(
            "không tạo được workspace {}",
            config.agent.workspace.display()
        )
    })?;
    let ws =
        CapWorkspace::open(config.agent.workspace.clone()).context("không mở được workspace")?;
    let ws: Arc<dyn beanagent_tools::WorkspaceFs> = Arc::new(ws);
    let mut registry = ToolRegistry::with_workspace(ws);

    // (M21.1) Workspace riêng cho từng project profile. Mỗi project có `MEMORY.md`/
    // `USER.md`/thư mục làm việc riêng ⇒ hai project không lẫn bộ nhớ hay ghi đè file
    // của nhau. Project `default` dùng sẵn `agent.workspace` nên không cần khai ở đây.
    let mut project_sandboxes: std::collections::BTreeMap<String, Arc<Sandbox>> =
        std::collections::BTreeMap::new();
    for profile in config.project_profiles() {
        if profile.name == beanagent_types::config::DEFAULT_PROJECT {
            continue;
        }
        std::fs::create_dir_all(&profile.workspace).with_context(|| {
            format!(
                "không tạo được workspace của project `{}` tại {}",
                profile.name,
                profile.workspace.display()
            )
        })?;
        let project_ws = CapWorkspace::open(profile.workspace.clone())
            .with_context(|| format!("không mở được workspace của project `{}`", profile.name))?;
        registry.set_project_workspace(&profile.name, Arc::new(project_ws));
        project_sandboxes.insert(
            profile.name.clone(),
            Arc::new(Sandbox::new(
                config.security.sandbox.clone(),
                profile.workspace.clone(),
            )),
        );
    }
    if config.tools.enabled.iter().any(|g| g == "files") {
        for tool in beanagent_tools::builtin::file_tools() {
            registry
                .register(tool)
                .context("đăng ký tool file thất bại")?;
        }
    }
    if config.tools.enabled.iter().any(|g| g == "shell") {
        let sandbox = Arc::new(Sandbox::new(
            config.security.sandbox.clone(),
            config.agent.workspace.clone(),
        ));
        registry
            .register(run_shell_for_projects(sandbox, project_sandboxes))
            .context("đăng ký run_shell thất bại")?;
    }
    if config.tools.enabled.iter().any(|g| g == "web") {
        let client = Arc::new(
            SafeHttpClient::new().context("không dựng được HTTP client an toàn cho web tools")?,
        );
        registry
            .register(web_fetch(client))
            .context("đăng ký web_fetch thất bại")?;
        registry
            .register(web_search(&config.tools.web_search, web_search_api_key)?)
            .context("đăng ký web_search thất bại")?;
    }
    if config.tools.enabled.iter().any(|g| g == "memory") {
        for tool in memory_tools(store.clone()) {
            registry
                .register(tool)
                .context("đăng ký tool memory thất bại")?;
        }
    }
    if config.tools.enabled.iter().any(|g| g == "skills") {
        for tool in skill_tools(skills) {
            registry
                .register(tool)
                .context("đăng ký tool skill thất bại")?;
        }
    }
    if config.tools.enabled.iter().any(|g| g == "schedule") {
        for tool in schedule_tools(
            store.clone(),
            config.agent.timezone.clone(),
            Arc::new(SystemClock),
        ) {
            registry
                .register(tool)
                .context("đăng ký tool scheduler thất bại")?;
        }
    }
    // (M22a) Domain tài chính read-only. Chỉ đăng ký khi `billing.enabled` **và** nhóm tool
    // `billing` bật; credential đọc từ biến môi trường riêng (validate đã chặn dùng chung).
    if config.billing.enabled && config.tools.enabled.iter().any(|g| g == "billing") {
        let billing_key = config
            .resolve_billing_key()
            .context("đọc credential billing thất bại")?;
        let client = BillingClient::new(&config.billing, billing_key);
        let configured = client.is_configured();
        if !configured {
            tracing::warn!(
                "billing đang bật nhưng thiếu [billing].base_url hoặc biến credential — \
                 tool sẽ chạy ở chế độ stub, KHÔNG gọi được nhà cung cấp cloud"
            );
        }
        registry
            .register(billing_read_cost(Arc::new(client), configured))
            .context("đăng ký tool billing thất bại")?;
    }
    // (M23) Domain quét bảo mật. Chỉ đăng ký khi `security_scan.enabled`; scope lấy từ
    // `[[infra_scope]]` (rỗng ⇒ mọi lần quét bị từ chối ở tầng code, xem D14.1).
    if config.security_scan.enabled {
        let scope = ScanScope::from_config(&config.infra_scope);
        if scope.is_empty() {
            tracing::warn!(
                "security_scan đang bật nhưng [[infra_scope]] rỗng — MỌI lần quét sẽ bị từ chối. \
                 Thêm target vào BeanAgent.toml trước khi dùng."
            );
        }
        // Sandbox RIÊNG cho scanner: khác hẳn `[security.sandbox]` của run_shell vì scanner
        // bắt buộc phải có mạng để tới target (D14.3).
        let scan_sandbox = Arc::new(Sandbox::new(
            config.security_scan.sandbox.to_sandbox_config(),
            config.agent.workspace.clone(),
        ));
        registry
            .register(security_scan(scope, scan_sandbox, ScannerCmd::default()))
            .context("đăng ký tool security_scan thất bại")?;
    }
    // (M24) Domain marketing. Chỉ đăng ký khi `marketing.enabled`; credential đọc từ
    // biến môi trường RIÊNG (validate chặn dùng chung với LLM/search/Telegram).
    if config.marketing.enabled {
        let key = config
            .resolve_marketing_key()
            .context("đọc credential marketing thất bại")?;
        let client = PublishClient::new(&config.marketing, key);
        if !client.is_configured() {
            tracing::warn!(
                "marketing đang bật nhưng thiếu [marketing].base_url hoặc biến credential — \
                 marketing_publish sẽ ở CHẾ ĐỘ STUB, KHÔNG đăng được gì"
            );
        }
        registry
            .register(marketing_publish(client))
            .context("đăng ký tool marketing_publish thất bại")?;
        registry
            .register(marketing_draft())
            .context("đăng ký tool marketing_draft thất bại")?;
    }
    let mcp = McpRuntime::load(&config.mcp_servers, &mut registry).await;
    Ok(BuiltRegistry { registry, mcp })
}

/// Hàng đợi stdin cho chế độ pipe; confirm và lượt user dùng chung một nguồn.
type SharedLines = Arc<tokio::sync::Mutex<VecDeque<String>>>;

async fn read_piped_lines() -> Result<SharedLines> {
    use tokio::io::AsyncReadExt;
    let mut input = String::new();
    tokio::io::stdin()
        .read_to_string(&mut input)
        .await
        .context("đọc stdin thất bại")?;
    Ok(Arc::new(tokio::sync::Mutex::new(
        input.lines().map(str::to_string).collect(),
    )))
}

struct CliChannel {
    piped: Option<SharedLines>,
}

impl CliChannel {
    fn new(piped: Option<SharedLines>) -> Self {
        Self { piped }
    }
}

#[async_trait::async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &'static str {
        "cli"
    }

    async fn run(&self, router: Arc<Router>, shutdown: CancellationToken) -> anyhow::Result<()> {
        match &self.piped {
            Some(lines) => run_piped(router, lines.clone(), shutdown).await,
            None => run_interactive(router, shutdown).await,
        }
    }

    async fn send(&self, _chat_id: &str, out: Outbound) -> anyhow::Result<()> {
        println!("[thông báo] {}", out.text);
        Ok(())
    }
}

enum CliEventAction {
    Confirm {
        id: String,
        prompt: String,
        allow_session: bool,
    },
    Done,
    Ignore,
}

fn event_run_id(event: &RunEvent) -> &RunId {
    match event {
        RunEvent::Queued { run_id, .. }
        | RunEvent::Text { run_id, .. }
        | RunEvent::TextDelta { run_id, .. }
        | RunEvent::ToolStart { run_id, .. }
        | RunEvent::ToolEnd { run_id, .. }
        | RunEvent::ConfirmRequest { run_id, .. }
        | RunEvent::ConfirmResolved { run_id, .. }
        | RunEvent::Final { run_id, .. }
        | RunEvent::Error { run_id, .. } => run_id,
    }
}

fn render_event(event: RunEvent, expected: &RunId) -> CliEventAction {
    if event_run_id(&event) != expected {
        return CliEventAction::Ignore;
    }
    match event {
        RunEvent::Queued { position, .. } => {
            println!("[queued] vị trí {position}");
            CliEventAction::Ignore
        }
        RunEvent::Text { text, .. } => {
            print!("{text}");
            CliEventAction::Ignore
        }
        // CLI giữ hành vi in một lần ở Final; Web mới dùng từng delta.
        RunEvent::TextDelta { .. } => CliEventAction::Ignore,
        RunEvent::ToolStart {
            tool,
            summary,
            args_preview,
            ..
        } => {
            println!("[tool] {tool}: {summary} ({args_preview})");
            CliEventAction::Ignore
        }
        RunEvent::ToolEnd {
            tool,
            ok,
            output_preview,
            ..
        } => {
            if ok {
                println!("[tool] {tool}: OK");
                if !output_preview.is_empty() {
                    println!("{output_preview}");
                }
            } else {
                eprintln!("[tool] {tool}: LỖI — {output_preview}");
            }
            CliEventAction::Ignore
        }
        RunEvent::ConfirmRequest {
            confirm_id,
            prompt,
            allow_session_option,
            ..
        } => CliEventAction::Confirm {
            id: confirm_id.to_string(),
            prompt,
            allow_session: allow_session_option,
        },
        RunEvent::ConfirmResolved { outcome, .. } => {
            println!("[xác nhận] {outcome:?}");
            CliEventAction::Ignore
        }
        RunEvent::Final { text, .. } => {
            if !text.is_empty() {
                println!("{text}");
            }
            CliEventAction::Done
        }
        RunEvent::Error { code, message, .. } => {
            eprintln!("lỗi agent ({code}): {message}");
            CliEventAction::Done
        }
    }
}

fn parse_decision(answer: &str, allow_session: bool) -> Decision {
    match answer.trim().to_lowercase().as_str() {
        "y" | "yes" => Decision::Allow,
        "s" | "session" | "allow-in-session" if allow_session => Decision::AllowInSession,
        _ => Decision::Deny,
    }
}

/// Vòng REPL tương tác: Ctrl-C khi chờ run sẽ chỉ cancel run đang chạy.
async fn run_interactive(router: Arc<Router>, shutdown: CancellationToken) -> Result<()> {
    let mut editor = DefaultEditor::new().context("không khởi tạo được terminal")?;
    let mut events = router.events();
    let mut active: Option<RunId> = None;

    'repl: loop {
        if active.is_none() {
            let line = match editor.readline(PROMPT) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    println!("(Ctrl-C) kết thúc phiên");
                    break;
                }
                Err(ReadlineError::Eof) => break,
                Err(error) => {
                    eprintln!("lỗi terminal: {error}");
                    break;
                }
            };
            let text = line.trim();
            if text.is_empty() {
                continue;
            }
            if matches!(text, "/exit" | "/quit") {
                break;
            }
            let _ = editor.add_history_entry(text);
            let run = router
                .submit(Incoming::new("cli", "local", "cli:local", text))
                .await
                .context("Router từ chối input CLI")?;
            active = Some(run);
            continue;
        }

        let Some(run) = active.as_ref() else {
            continue;
        };
        let event = tokio::select! {
            _ = shutdown.cancelled() => {
                router.cancel("cli", "local").await;
                break 'repl;
            }
            signal = tokio::signal::ctrl_c() => {
                if signal.is_ok() {
                    println!("(Ctrl-C) đang dừng run");
                    router.cancel("cli", "local").await;
                    continue;
                }
                eprintln!("không lắng nghe được Ctrl-C");
                break 'repl;
            }
            event = router.recv_event(&mut events) => {
                match event {
                    Some(event) => event,
                    None => break 'repl,
                }
            }
        };
        match render_event(event, run) {
            CliEventAction::Ignore => {}
            CliEventAction::Done => active = None,
            CliEventAction::Confirm {
                id,
                prompt,
                allow_session,
            } => {
                println!("[xác nhận] {prompt}");
                let suffix = if allow_session { "y/n/s" } else { "y/n" };
                match editor.readline(&format!("Cho phép? ({suffix}): ")) {
                    Ok(answer) => {
                        let decision = parse_decision(&answer, allow_session);
                        if let Err(error) = router.resolve_confirm(&id, decision, "cli:local").await
                        {
                            eprintln!("không resolve confirm: {error}");
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        router.cancel("cli", "local").await;
                    }
                    Err(ReadlineError::Eof) => {
                        let _ = router
                            .resolve_confirm(&id, Decision::Deny, "cli:local")
                            .await;
                    }
                    Err(error) => {
                        eprintln!("lỗi đọc xác nhận: {error}");
                        let _ = router
                            .resolve_confirm(&id, Decision::Deny, "cli:local")
                            .await;
                    }
                }
            }
        }
    }
    router.cancel("cli", "local").await;
    Ok(())
}

/// Chế độ pipe: mỗi dòng là input; dòng kế tiếp sau ConfirmRequest là câu trả lời.
async fn run_piped(
    router: Arc<Router>,
    lines: SharedLines,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut events = router.events();
    let mut active: Option<RunId> = None;

    loop {
        if active.is_none() {
            let line = tokio::select! {
                _ = shutdown.cancelled() => None,
                signal = tokio::signal::ctrl_c() => {
                    if signal.is_err() {
                        eprintln!("không lắng nghe được Ctrl-C");
                    }
                    None
                }
                line = async { lines.lock().await.pop_front() } => line,
            };
            let Some(line) = line else { break };
            let text = line.trim();
            if text.is_empty() {
                continue;
            }
            if matches!(text, "/exit" | "/quit") {
                break;
            }
            let run = router
                .submit(Incoming::new("cli", "local", "cli:local", text))
                .await
                .context("Router từ chối input CLI")?;
            active = Some(run);
            continue;
        }

        let Some(run) = active.as_ref() else {
            continue;
        };
        let event = tokio::select! {
            _ = shutdown.cancelled() => {
                router.cancel("cli", "local").await;
                break;
            }
            signal = tokio::signal::ctrl_c() => {
                if signal.is_ok() {
                    router.cancel("cli", "local").await;
                    continue;
                }
                break;
            }
            event = router.recv_event(&mut events) => {
                match event {
                    Some(event) => event,
                    None => break,
                }
            }
        };
        match render_event(event, run) {
            CliEventAction::Ignore => {}
            CliEventAction::Done => active = None,
            CliEventAction::Confirm {
                id,
                prompt,
                allow_session,
            } => {
                println!("[xác nhận] {prompt}");
                let answer = lines.lock().await.pop_front().unwrap_or_default();
                let decision = parse_decision(&answer, allow_session);
                if let Err(error) = router.resolve_confirm(&id, decision, "cli:local").await {
                    eprintln!("không resolve confirm: {error}");
                }
            }
        }
    }
    router.cancel("cli", "local").await;
    Ok(())
}
