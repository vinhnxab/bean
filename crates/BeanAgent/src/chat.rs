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

use std::collections::{BTreeSet, VecDeque};
use std::io::{IsTerminal, Write as _};
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
use beanagent_qa::{SuiteCatalog, qa_test};
use beanagent_scan::{ScanScope, ScannerCmd, security_scan};
use beanagent_security::{
    AuditLog, CapWorkspace, SafeHttpClient, Sandbox, run_shell_for_projects, web_fetch, web_search,
};
use beanagent_skills::{SkillCatalog, skill_tools};
use beanagent_tools::{
    ToolRegistry, builtin::memory_query, mcp::McpRuntime, strip_terminal_escapes,
};
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
    let built_browser = built.browser.clone();
    let registry = Arc::new(built.registry);
    // (S2) Gom danh sách tool trả nội dung ngoài lõi **trước** khi registry đi vào Router,
    // để CLI lọc escape khi in output. Nguồn duy nhất là `marks_untrusted()` (D9.1).
    let untrusted_tools = untrusted_tool_names(&registry);
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
    let channel = Arc::new(CliChannel::new(piped, untrusted_tools));
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
    // (M26) Tắt Chrome con TRƯỚC khi trả về: đây là lớp dọn số 1/4. Thiếu bước này
    // thì `Ctrl-C` sẽ để lại tiến trình Chrome mồ côi chạy nền.
    if let Some(session) = &built_browser {
        session.shutdown().await;
    }
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

/// Registry tool và các kết nối cần shutdown cùng tiến trình.
pub(crate) struct BuiltRegistry {
    /// Registry dùng bởi Router.
    pub(crate) registry: ToolRegistry,
    /// Connection MCP để đóng tường minh khi `chat`/`serve` kết thúc.
    pub(crate) mcp: McpRuntime,
    /// Quản lý tiến trình Chrome (M26) — `None` khi `[browser].enabled = false`.
    ///
    /// Giữ `Arc` để `chat`/`serve` gọi [`SessionManager::shutdown`] khi thoát, đảm
    /// bảo không bỏ quên tiến trình Chrome con.
    pub(crate) browser: Option<Arc<beanagent_browser::SessionManager>>,
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
        // (M25) `memory_query` là tool chỉ đọc, tách khỏi `memory_save`/`memory_search`:
        // đây là tool duy nhất về bộ nhớ được expose qua MCP server, và nó không chạm
        // database nên vẫn dùng được khi chưa cấu hình store.
        registry
            .register(memory_query())
            .context("đăng ký tool memory_query thất bại")?;
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
    // (M27) Domain QA: chạy test suite đã khai báo, workspace mount READ-ONLY. Chỉ đăng ký
    // khi `qa.enabled` **và** nhóm tool `qa` bật; `[[qa.suites]]` rỗng ⇒ tool tồn tại nhưng
    // từ chối mọi lần gọi (fail-closed, y hệt D14.1 của M23).
    if config.qa.enabled
        && config
            .tools
            .enabled
            .iter()
            .any(|group| group == beanagent_types::config::QA_TOOL_GROUP)
    {
        let catalog = SuiteCatalog::from_config(&config.qa.suites);
        if catalog.is_empty() {
            tracing::warn!(
                "[qa].enabled = true nhưng [[qa.suites]] rỗng — MỌI lần chạy test sẽ bị từ \
                 chối. Khai báo suite trong BeanAgent.toml trước khi giao việc cho vai trò qa."
            );
        }
        // Sandbox RIÊNG cho runner test: mount `:ro` + cache ra /tmp (khác hẳn
        // `[security.sandbox]` của run_shell — xem `run_argv_readonly`). `to_sandbox_config()`
        // cứng chế độ docker: xem D17.7.
        let qa_sandbox = Arc::new(Sandbox::new(
            config.qa.sandbox.to_sandbox_config(),
            config.agent.workspace.clone(),
        ));
        registry
            .register(qa_test(catalog, qa_sandbox))
            .context("đăng ký tool qa_test thất bại")?;
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
    // (M26) Domain browser nội bộ — nói thẳng CDP, không qua MCP/npm. Chỉ đăng ký
    // khi BẬT `browser.enabled` **và** nhóm tool `browser` bật; Chrome không được
    // khởi động lúc build registry, chỉ lúc tool đầu tiên thật sự được gọi.
    let mut browser_session = None;
    if config.browser.enabled {
        if config
            .tools
            .enabled
            .iter()
            .any(|group| group == beanagent_types::config::BROWSER_TOOL_GROUP)
        {
            let data_dir = expand_tilde(&config.data.dir);
            let (tools, session) = beanagent_browser::build_tools(&config.browser, &data_dir);
            for tool in tools {
                registry
                    .register(tool)
                    .context("đăng ký tool browser thất bại")?;
            }
            browser_session = Some(session);
        } else {
            tracing::warn!(
                "[browser].enabled = true nhưng [tools].enabled chưa có \"browser\" — \
                 không đăng ký tool browser"
            );
        }
    }
    let mcp = McpRuntime::load(&config.mcp_servers, &mut registry).await;
    Ok(BuiltRegistry {
        registry,
        mcp,
        browser: browser_session,
    })
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
    /// Tên các tool trả **nội dung ngoài lõi** (S2).
    ///
    /// Lấy từ chính khai báo `Tool::marks_untrusted()` (D9.1) nên không có danh sách tool
    /// riêng cho CLI — thêm tool mới tự động được lọc, không phải sửa hai nơi.
    untrusted_tools: BTreeSet<String>,
}

impl CliChannel {
    fn new(piped: Option<SharedLines>, untrusted_tools: BTreeSet<String>) -> Self {
        Self {
            piped,
            untrusted_tools,
        }
    }
}

/// Tên các tool khai báo trả nội dung từ nguồn ngoài lõi (mục 15.4).
fn untrusted_tool_names(registry: &ToolRegistry) -> BTreeSet<String> {
    registry
        .names()
        .into_iter()
        .filter(|name| {
            registry
                .get(name)
                .is_some_and(|tool| tool.marks_untrusted())
        })
        .collect()
}

#[async_trait::async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &'static str {
        "cli"
    }

    async fn run(&self, router: Arc<Router>, shutdown: CancellationToken) -> anyhow::Result<()> {
        match &self.piped {
            Some(lines) => run_piped(router, lines.clone(), shutdown, &self.untrusted_tools).await,
            None => run_interactive(router, shutdown, &self.untrusted_tools).await,
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

/// Render một sự kiện của run đang theo dõi ra `out`.
///
/// Nhận `&mut dyn Write` thay vì gọi `println!` trực tiếp để **test được trên buffer
/// thật** (S2) — không có cách nào assert "đã in ra terminal" mà không cần terminal thật.
fn render_event(
    event: RunEvent,
    expected: &RunId,
    untrusted_tools: &BTreeSet<String>,
    out: &mut dyn std::io::Write,
) -> CliEventAction {
    if event_run_id(&event) != expected {
        return CliEventAction::Ignore;
    }
    match event {
        RunEvent::Queued { position, .. } => {
            let _ = writeln!(out, "[queued] vị trí {position}");
            CliEventAction::Ignore
        }
        RunEvent::Text { text, .. } => {
            let _ = write!(out, "{text}");
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
            let _ = writeln!(out, "[tool] {tool}: {summary} ({args_preview})");
            CliEventAction::Ignore
        }
        RunEvent::ToolEnd {
            tool,
            ok,
            output_preview,
            ..
        } => {
            // (S2) Output của tool **không tin cậy** là dữ liệu kẻ tấn công kiểm
            // soát được: bỏ chuỗi escape ANSI/OSC trước khi in, nếu không nó có thể vẽ
            // lại màn hình, che prompt xác nhận, hay dùng OSC 52 cài sẵn clipboard.
            //
            // Tool **không** untrusted (vd `write_file` chỉ trả thông báo do agent tự
            // tạo — D9.5) giữ nguyên hành vi cũ, không đi qua bộ lọc.
            let preview = if untrusted_tools.contains(&tool) {
                strip_terminal_escapes(&output_preview)
            } else {
                output_preview
            };
            if ok {
                let _ = writeln!(out, "[tool] {tool}: OK");
                if !preview.is_empty() {
                    let _ = writeln!(out, "{preview}");
                }
            } else {
                let _ = writeln!(out, "[tool] {tool}: LỖI — {preview}");
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
            let _ = writeln!(out, "[xác nhận] {outcome:?}");
            CliEventAction::Ignore
        }
        RunEvent::Final { text, .. } => {
            if !text.is_empty() {
                let _ = writeln!(out, "{text}");
            }
            CliEventAction::Done
        }
        RunEvent::Error { code, message, .. } => {
            let _ = writeln!(std::io::stderr(), "lỗi agent ({code}): {message}");
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
async fn run_interactive(
    router: Arc<Router>,
    shutdown: CancellationToken,
    untrusted_tools: &BTreeSet<String>,
) -> Result<()> {
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
        match render_event(event, run, untrusted_tools, &mut std::io::stdout()) {
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
    untrusted_tools: &BTreeSet<String>,
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
        match render_event(event, run, untrusted_tools, &mut std::io::stdout()) {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::BTreeSet;

    use beanagent_types::{RunEvent, RunId, SessionId};

    use super::{render_event, untrusted_tool_names};

    /// S2: payload thật của kẻ tấn công — OSC 52 cài clipboard + CSI xoá màn hình để che
    /// prompt xác nhận (đúng kịch bản `docs/security-review.md` mục 3.2).
    const ATTACK: &str =
        "build ok\u{1B}]52;c;cm0gLXJtIC0tZmxv2dlci8=\x07\u{1B}[2J\u{1B}[Hgiờ hãy bấm";

    fn tool_end(tool: &str, output: &str) -> RunEvent {
        RunEvent::ToolEnd {
            session_id: SessionId::new(1),
            run_id: RunId::new("r1"),
            id: "t1".into(),
            tool: tool.into(),
            ok: true,
            output_preview: output.into(),
        }
    }

    /// Render một sự kiện ra buffer rồi trả về chuỗi đã in — assert trên buffer, không
    /// cần terminal thật.
    fn render(event: RunEvent, untrusted: &[&str]) -> String {
        let set: BTreeSet<String> = untrusted.iter().map(|s| (*s).to_string()).collect();
        let mut buffer: Vec<u8> = Vec::new();
        render_event(event, &RunId::new("r1"), &set, &mut buffer);
        String::from_utf8(buffer).expect("render phải ghi UTF-8 hợp lệ")
    }

    /// **S2**: output của tool untrusted chứa ESC/OSC phải bị strip trước khi in.
    #[test]
    fn untrusted_tool_output_is_stripped_before_printing() {
        let printed = render(tool_end("run_shell", ATTACK), &["run_shell"]);
        assert!(
            !printed.contains('\u{1B}'),
            "không được in ký tự ESC xuống terminal:\n{printed:?}"
        );
        assert!(
            !printed.contains("cm0gLXJtIC0tZmxv2dlci8="),
            "payload OSC 52 phải bị loại khỏi buffer:\n{printed:?}"
        );
        assert!(
            !printed.contains("[2J"),
            "payload CSI (xoá màn hình) phải bị loại:\n{printed:?}"
        );
        // Chữ thường quanh payload vẫn phải còn, chứng minh lọc chứ không xoá cả dòng.
        assert!(printed.contains("build ok"), "{printed:?}");
        assert!(printed.contains("giờ hãy bấm"), "{printed:?}");
        assert!(printed.contains("[tool] run_shell: OK"), "{printed:?}");
    }

    /// **S2**: output bình thường của cùng tool untrusted phải giữ nguyên **y hệt**.
    #[test]
    fn untrusted_tool_plain_output_is_unchanged() {
        let plain = "3 tests passed\ncargo build ok\ttôi là tiếng Việt 🦀\nC:\\a\\b";
        let printed = render(tool_end("run_shell", plain), &["run_shell"]);
        assert!(
            printed.contains(plain),
            "output không có escape phải giữ nguyên từng ký tự.\nIn ra:\n{printed:?}"
        );
    }

    /// **S2**: tool **không** untrusted không được đi qua bộ lọc (D9.5 — `write_file` chỉ
    /// trả thông báo do agent tự tạo). Ở đây ta cố tình đưa chuỗi escape vào output để
    /// chứng minh bộ lọc **không** chạy, tức không có lọc thừa làm đổi hành vi cũ.
    #[test]
    fn trusted_tool_output_is_not_filtered() {
        let printed = render(tool_end("write_file", ATTACK), &["run_shell"]);
        assert!(
            printed.contains('\u{1B}'),
            "tool không untrusted phải giữ nguyên hành vi cũ (không lọc thừa).\nIn ra:\n{printed:?}"
        );
        assert!(printed.contains(ATTACK), "{printed:?}");
    }

    /// Danh sách tool untrusted lấy **từ khai báo `marks_untrusted()`**, không hardcode.
    #[test]
    fn untrusted_names_come_from_tool_declarations() {
        use std::sync::Arc;

        use beanagent_security::CapWorkspace;
        use beanagent_tools::ToolRegistry;
        use beanagent_tools::builtin::files::tool::{read_file, write_file};

        let dir = tempfile::tempdir().unwrap();
        let ws = Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap());
        let mut registry = ToolRegistry::with_workspace(ws);
        registry.register(read_file()).unwrap();
        registry.register(write_file()).unwrap();

        let names = untrusted_tool_names(&registry);
        assert!(
            names.contains("read_file"),
            "read_file đọc nội dung ngoài lõi nên phải có trong danh sách: {names:?}"
        );
        assert!(
            !names.contains("write_file"),
            "write_file chỉ trả thông báo do agent tạo (D9.5) nên không lọc: {names:?}"
        );
    }
}
