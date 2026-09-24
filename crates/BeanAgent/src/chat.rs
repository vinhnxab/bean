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

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use beanagent_core::{Decision, RunIo, RunTurnArgs, SqliteStore, Store, memory_tools, run_turn};
use beanagent_llm::{FakeProvider, LlmProvider};
use beanagent_security::{
    AuditLog, CapWorkspace, SafeHttpClient, Sandbox, SessionPolicy, run_shell, web_fetch,
    web_search,
};
use beanagent_skills::{SkillCatalog, skill_tools};
use beanagent_tools::ToolRegistry;
use beanagent_types::config::Config;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use crate::cli::ChatArgs;

const PROMPT: &str = "bạn> ";

/// Kết nối bảo-security của phiên CLI (M4): allow-in-session + audit log.
struct CliSecurity {
    session_policy: Arc<SessionPolicy>,
    audit: Option<Arc<AuditLog>>,
}

/// Dựng security cho phiên CLI: session policy (cho phép "trong phiên" qua các turn)
/// và audit log trong `<data.dir>/audit/audit.jsonl` (mục 15.8).
fn build_security(config: &Config) -> CliSecurity {
    let session_policy = Arc::new(SessionPolicy::new());
    let audit_dir = expand_tilde(&config.data.dir).join("audit");
    let audit = match AuditLog::open(&audit_dir) {
        Ok(log) => Some(Arc::new(log)),
        Err(err) => {
            // Không có audit không được chặn phiên demo — chỉ cảnh báo rõ ràng.
            eprintln!(
                "cảnh báo: không mở được audit log ở {}: {err}",
                audit_dir.display()
            );
            None
        }
    };
    CliSecurity {
        session_policy,
        audit,
    }
}

/// Mở rộng `~` trong đường dẫn cấu hình bằng biến môi trường `HOME`.
fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

/// Đường dẫn đến SQLite database.
fn store_path(config: &Config) -> PathBuf {
    expand_tilde(&config.data.dir).join("beanagent.db")
}

/// Chạy REPL.
///
/// # Errors
/// Lỗi khi nạp cấu hình/kịch bản/secret, hoặc khi không mở được terminal.
pub async fn run(args: &ChatArgs, config_path: Option<&Path>) -> Result<()> {
    let mut config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    // `--workspace` ghi đè `[agent] workspace` (tiện cho demo/test).
    if let Some(ws) = &args.workspace {
        config.agent.workspace = ws.clone();
    }
    let (provider, web_search_api_key) = build_provider(args, &config)?;
    let store = Arc::new(
        SqliteStore::open(&store_path(&config))
            .map_err(|e| anyhow::anyhow!("không mở được store: {e}"))?,
    );
    let user_skills_root = config.data.dir.join("skills");
    let skills = SkillCatalog::load_with_create_root(
        &[PathBuf::from("skills"), user_skills_root.clone()],
        user_skills_root,
    );
    let registry = build_registry(&config, store.clone(), skills.clone(), web_search_api_key)?;
    let security = build_security(&config);
    let cancel = CancellationToken::new();

    println!(
        "BeanAgent chat — provider: {}. Gõ /exit hoặc Ctrl-D để thoát.",
        provider.name()
    );
    println!("workspace: {}", config.agent.workspace.display());

    if std::io::stdin().is_terminal() {
        run_interactive(
            &config,
            &provider,
            &registry,
            &skills,
            store,
            cancel.clone(),
            &security,
        )
        .await?;
    } else {
        run_piped(
            &config,
            &provider,
            &registry,
            &skills,
            store,
            cancel.clone(),
            &security,
        )
        .await?;
    }
    Ok(())
}

/// Chọn provider: kịch bản `--fake-llm` nếu có, ngược lại provider thật theo `[llm]`.
///
/// Lỗi trả về luôn nêu rõ **nguyên nhân cấu hình** (thiếu biến môi trường, sai provider…)
/// chứ không lộ giá trị secret.
fn build_provider(
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

/// Xây registry tool từ cấu hình — tự tạo `agent.workspace` nếu chưa tồn tại (mục 4).
/// Path jail bằng `CapWorkspace` (cap-std — mục 15.1); `run_shell` gắn sandbox
/// docker/host (mục 15.2).
fn build_registry(
    config: &Config,
    store: Arc<SqliteStore>,
    skills: SkillCatalog,
    web_search_api_key: Option<SecretString>,
) -> Result<ToolRegistry> {
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
            .register(run_shell(sandbox))
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
        for tool in memory_tools(store) {
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
    Ok(registry)
}

/// `/exit` và `/quit` kết thúc phiên (M8 sẽ có bộ slash command đầy đủ ở lõi).
fn is_exit_command(text: &str) -> bool {
    matches!(text, "/exit" | "/quit")
}

/// Vòng REPL tương tác (rustyline).
async fn run_interactive(
    config: &Config,
    provider: &Arc<dyn LlmProvider>,
    registry: &ToolRegistry,
    skills: &SkillCatalog,
    store: Arc<SqliteStore>,
    cancel: CancellationToken,
    security: &CliSecurity,
) -> Result<()> {
    let io = CliIo::interactive();
    let mut editor = DefaultEditor::new().context("không khởi tạo được terminal")?;
    let mut session = store
        .ensure_session("cli", "local", "")
        .await
        .map_err(|e| anyhow::anyhow!("không tạo được session: {e}"))?;
    let store_ref: &dyn Store = &*store as &dyn Store;

    loop {
        let line = match editor.readline(PROMPT) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => {
                println!("(Ctrl-C) kết thúc phiên");
                break;
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("lỗi terminal: {err}");
                break;
            }
        };

        let text = line.trim().to_string();
        if text.is_empty() {
            continue;
        }
        if text == "/new" {
            if session.get() > 0 {
                let _ = store.archive_session(session).await;
            }
            session = store
                .ensure_session("cli", "local", "")
                .await
                .map_err(|e| anyhow::anyhow!("không tạo session mới: {e}"))?;
            println!("→ Phiên mới đã tạo.");
            continue;
        }
        if is_exit_command(&text) {
            break;
        }
        // Lỗi lịch sử không làm hỏng phiên chat.
        let _ = editor.add_history_entry(&text);

        let io_ref = &io;
        let skills_index = if config.tools.enabled.iter().any(|group| group == "skills") {
            skills.index()
        } else {
            String::new()
        };
        let result = run_turn(RunTurnArgs {
            store: store_ref,
            registry,
            llm: provider.as_ref(),
            config,
            session,
            user_text: text,
            io: io_ref,
            cancel: cancel.clone(),
            session_policy: Some(security.session_policy.clone()),
            audit: security.audit.clone(),
            channel: "cli",
            skills_index: &skills_index,
        })
        .await;

        match result {
            Ok(final_text) => {
                if !final_text.is_empty() {
                    println!("{final_text}");
                }
            }
            Err(err) => {
                eprintln!("lỗi agent: {err}");
            }
        }
    }

    Ok(())
}

/// Chế độ không có TTY: đọc **toàn bộ** stdin trước vào hàng đợi dùng chung
/// (REPL và `confirm` cùng lấy từ đó — xem `SharedLines`), chạy tới hết hoặc `/exit`.
async fn run_piped(
    config: &Config,
    provider: &Arc<dyn LlmProvider>,
    registry: &ToolRegistry,
    skills: &SkillCatalog,
    store: Arc<SqliteStore>,
    cancel: CancellationToken,
    security: &CliSecurity,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    let mut buf = String::new();
    tokio::io::stdin()
        .read_to_string(&mut buf)
        .await
        .context("đọc stdin thất bại")?;
    let queue: SharedLines = Arc::new(tokio::sync::Mutex::new(
        buf.lines().map(str::to_string).collect(),
    ));
    let io = CliIo::piped(queue.clone());
    let mut session = store
        .ensure_session("cli", "local", "")
        .await
        .map_err(|e| anyhow::anyhow!("không tạo được session: {e}"))?;
    let store_ref: &dyn Store = &*store as &dyn Store;

    loop {
        let line = {
            // Guard của khoá phải được thả **trước khi** chạy run_turn: `confirm` của
            // CliIo trong turn cũng lấy khoá này. Nếu giữ guard qua `.await` của thân
            // vòng lặp (temporary của scrutinee `while let` sống hết thân) sẽ deadlock.
            match queue.lock().await.pop_front() {
                Some(l) => l,
                None => break,
            }
        };
        let text = line.trim().to_string();
        if text.is_empty() {
            continue;
        }
        if text == "/new" {
            if session.get() > 0 {
                let _ = store.archive_session(session).await;
            }
            session = store
                .ensure_session("cli", "local", "")
                .await
                .map_err(|e| anyhow::anyhow!("không tạo session mới: {e}"))?;
            println!("→ Phiên mới đã tạo.");
            continue;
        }
        if is_exit_command(&text) {
            break;
        }

        let io_ref = &io;
        let skills_index = if config.tools.enabled.iter().any(|group| group == "skills") {
            skills.index()
        } else {
            String::new()
        };
        let result = run_turn(RunTurnArgs {
            store: store_ref,
            registry,
            llm: provider.as_ref(),
            config,
            session,
            user_text: text,
            io: io_ref,
            cancel: cancel.clone(),
            session_policy: Some(security.session_policy.clone()),
            audit: security.audit.clone(),
            channel: "cli",
            skills_index: &skills_index,
        })
        .await;

        match result {
            Ok(final_text) => {
                if !final_text.is_empty() {
                    println!("{final_text}");
                }
            }
            Err(err) => {
                eprintln!("lỗi agent: {err}");
            }
        }
    }

    Ok(())
}

/// Hàng đợi dòng stdin dùng chung cho REPL và `confirm` trong chế độ pipe.
///
/// `BufReader` của REPL đọc trước được nhiều dòng vào buffer nội bộ, khiến
/// `confirm` đọc thẳng `stdin` thấy EOF và luôn từ chối (mục 22). Đọc toàn bộ
/// stdin một lần rồi chia qua hàng đợi dùng chung để hai bên không giành nhau.
type SharedLines = Arc<tokio::sync::Mutex<std::collections::VecDeque<String>>>;

/// Implement `RunIo` cho CLI.
struct CliIo {
    /// `Some` khi chạy ở chế độ pipe: confirm lấy câu trả lời từ hàng đợi thay vì stdin.
    piped: Option<SharedLines>,
}

impl CliIo {
    fn interactive() -> Self {
        Self { piped: None }
    }

    fn piped(lines: SharedLines) -> Self {
        Self { piped: Some(lines) }
    }
}

#[async_trait::async_trait]
impl RunIo for CliIo {
    fn on_text(&self, text: &str) {
        print!("{text}");
    }

    fn on_tool_start(&self, tool: &str, summary: &str, args: &str) {
        println!("[tool] {tool}: {summary} ({args})");
    }

    fn on_tool_end(&self, tool: &str, ok: bool, output: &str) {
        if ok {
            println!("[tool] {tool}: OK");
            if !output.is_empty() {
                println!("{output}");
            }
        } else {
            eprintln!("[tool] {tool}: LỖI — {output}");
        }
    }

    async fn confirm(
        &self,
        prompt: &str,
        allow_in_session: bool,
        _timeout: std::time::Duration,
    ) -> Option<Decision> {
        println!("[xác nhận] {prompt}");
        // Chỉ hiện tuỳ chọn "s" khi policy cho phép (Confirm + chưa untrusted — mục 7.2).
        if allow_in_session {
            print!("Cho phép? (y/n/s): ");
        } else {
            print!("Cho phép? (y/n): ");
        }
        use std::io::Write;
        let _ = std::io::stdout().flush();

        let line = if let Some(queue) = &self.piped {
            queue.lock().await.pop_front()
        } else {
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok()?;
            Some(input)
        };
        match line?.trim().to_lowercase().as_str() {
            "y" | "yes" => Some(Decision::Allow),
            // "s" luôn trả AllowInSession — core chỉ persist khi policy cho phép.
            "s" | "session" | "allow-in-session" => Some(Decision::AllowInSession),
            _ => Some(Decision::Deny),
        }
    }

    fn cancel_token(&self) -> &CancellationToken {
        // CLI không có cancel token thực tế, dùng token không bao giờ bị huỷ
        use std::sync::OnceLock;
        static TOKEN: OnceLock<CancellationToken> = OnceLock::new();
        TOKEN.get_or_init(CancellationToken::new)
    }
}
