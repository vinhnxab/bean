//! `BeanAgent chat` — REPL tối giản.
//!
//! * ~~M1~~: `FakeProvider` (echo / kịch bản `--fake-llm`); chưa có agent loop.
//! * **M2**: provider thật theo `[llm]` qua `beanagent_llm::build_provider`
//!   (một lượt, chưa có tool). `--fake-llm` vẫn ghi đè lên provider thật — hữu ích
//!   cho demo và test end-to-end không cần mạng.
//! * **M3**: thêm agent loop + tool; **M8**: chuyển sang đi qua `Router` như một `Channel`
//!   (`channel = "cli"`, `chat_id = "local"`), Ctrl-C trở thành `cancel` của run đang chạy.
//!
//! Hai chế độ vào:
//! * có TTY → `rustyline` (lịch sử, sửa dòng);
//! * không có TTY (pipe/file) → đọc từng dòng từ stdin, phục vụ `echo "…" | BeanAgent chat`,
//!   test CLI và script end-to-end ở M16.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use beanagent_core::{Decision, RunIo, RunTurnArgs, run_turn};
use beanagent_llm::{FakeProvider, LlmProvider};
use beanagent_tools::{FsWorkspace, ToolRegistry};
use beanagent_types::config::Config;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use tokio_util::sync::CancellationToken;

use crate::cli::ChatArgs;

const PROMPT: &str = "bạn> ";

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
    let provider = build_provider(args, &config)?;
    let registry = build_registry(&config)?;
    let store = beanagent_core::store::MemoryStore::new();
    let cancel = CancellationToken::new();

    println!(
        "BeanAgent chat — provider: {}. Gõ /exit hoặc Ctrl-D để thoát.",
        provider.name()
    );
    println!("workspace: {}", config.agent.workspace.display());

    if std::io::stdin().is_terminal() {
        run_interactive(&config, &provider, &registry, store, cancel.clone()).await?;
    } else {
        run_piped(&config, &provider, &registry, store, cancel.clone()).await?;
    }
    Ok(())
}

/// Chọn provider: kịch bản `--fake-llm` nếu có, ngược lại provider thật theo `[llm]`.
///
/// Lỗi trả về luôn nêu rõ **nguyên nhân cấu hình** (thiếu biến môi trường, sai provider…)
/// chứ không lộ giá trị secret.
fn build_provider(args: &ChatArgs, config: &Config) -> Result<Arc<dyn LlmProvider>> {
    if let Some(path) = args.fake_llm.as_deref() {
        let provider =
            FakeProvider::from_json_path(path).context("nạp kịch bản --fake-llm thất bại")?;
        return Ok(Arc::new(provider));
    }
    let secrets = config
        .resolve_secrets()
        .context("đọc secret từ biến môi trường thất bại")?;
    beanagent_llm::build_provider(&config.llm, secrets.llm_api_key)
        .context("dựng provider LLM thất bại (kiểm tra [llm] trong BeanAgent.toml)")
}

/// Xây registry tool từ cấu hình.
///
/// Tự tạo `agent.workspace` nếu chưa tồn tại (agents.md mục 4).
fn build_registry(config: &Config) -> Result<ToolRegistry> {
    std::fs::create_dir_all(&config.agent.workspace).with_context(|| {
        format!(
            "không tạo được workspace {}",
            config.agent.workspace.display()
        )
    })?;
    let ws =
        FsWorkspace::open(config.agent.workspace.clone()).context("không mở được workspace")?;
    let ws: Arc<dyn beanagent_tools::WorkspaceFs> = Arc::new(ws);
    let mut registry = ToolRegistry::with_workspace(ws);
    if config.tools.enabled.iter().any(|g| g == "files") {
        for tool in beanagent_tools::builtin::file_tools() {
            registry
                .register(tool)
                .context("đăng ký tool file thất bại")?;
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
    store: beanagent_core::store::MemoryStore,
    cancel: CancellationToken,
) -> Result<()> {
    let io = CliIo::interactive();
    let mut editor = DefaultEditor::new().context("không khởi tạo được terminal")?;
    let session = beanagent_types::SessionId::from(1i64);

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
        if is_exit_command(&text) {
            break;
        }
        // Lỗi lịch sử không làm hỏng phiên chat.
        let _ = editor.add_history_entry(&text);

        let io_ref = &io;
        let result = run_turn(RunTurnArgs {
            store: &store,
            registry,
            llm: provider.as_ref(),
            config,
            session,
            user_text: text,
            io: io_ref,
            cancel: cancel.clone(),
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
    store: beanagent_core::store::MemoryStore,
    cancel: CancellationToken,
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
    let session = beanagent_types::SessionId::from(1i64);

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
        if is_exit_command(&text) {
            break;
        }

        let io_ref = &io;
        let result = run_turn(RunTurnArgs {
            store: &store,
            registry,
            llm: provider.as_ref(),
            config,
            session,
            user_text: text,
            io: io_ref,
            cancel: cancel.clone(),
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
        _allow_in_session: bool,
        _timeout: std::time::Duration,
    ) -> Option<Decision> {
        println!("[xác nhận] {prompt}");
        print!("Cho phép? (y/n/s): ");
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
