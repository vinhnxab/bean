//! `BeanAgent chat` — REPL tối giản.
//!
//! * **M1**: `FakeProvider` (echo khi chưa có API key, hoặc kịch bản `--fake-llm`); chưa có agent loop.
//! * **M2**: dùng provider thật theo `[llm]` (một lượt, chưa có tool).
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
use beanagent_llm::{ChatRequest, FakeProvider, LlmProvider};
use beanagent_types::Message;
use beanagent_types::config::Config;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use tokio::io::AsyncBufReadExt;

use crate::cli::ChatArgs;

const PROMPT: &str = "bạn> ";

/// System prompt tối thiểu cho M1; bản đầy đủ theo agents.md mục 19 được dựng ở M3.
const SYSTEM_PROMPT_M1: &str =
    "You are BeanAgent, a personal AI assistant running on the user's own machine.";

/// Chạy REPL.
///
/// # Errors
/// Lỗi khi nạp cấu hình/kịch bản, hoặc khi không mở được terminal.
pub async fn run(args: &ChatArgs, config_path: Option<&Path>) -> Result<()> {
    let config = Config::load_or_default(config_path).context("nạp cấu hình thất bại")?;
    let provider = build_provider(args)?;

    println!(
        "BeanAgent chat — provider: {}. Gõ /exit hoặc Ctrl-D để thoát.",
        provider.name()
    );
    println!("workspace: {}", config.agent.workspace.display());

    if std::io::stdin().is_terminal() {
        run_interactive(&config, &provider).await
    } else {
        run_piped(&config, &provider).await
    }
}

/// Chọn provider: kịch bản `--fake-llm` hoặc chế độ echo (M2 sẽ thêm provider thật).
fn build_provider(args: &ChatArgs) -> Result<Arc<dyn LlmProvider>> {
    match args.fake_llm.as_deref() {
        Some(path) => {
            let provider =
                FakeProvider::from_json_path(path).context("nạp kịch bản --fake-llm thất bại")?;
            Ok(Arc::new(provider))
        }
        None => {
            tracing::warn!("chưa cài provider thật (M2) — dùng chế độ echo của FakeProvider");
            Ok(Arc::new(FakeProvider::echo()))
        }
    }
}

/// Gửi đúng một lượt rồi in kết quả. Lỗi provider **không** làm hỏng phiên.
async fn respond(
    config: &Config,
    provider: &Arc<dyn LlmProvider>,
    messages: &mut Vec<Message>,
    text: &str,
) {
    messages.push(Message::user(text));

    let request = ChatRequest {
        system: SYSTEM_PROMPT_M1,
        messages,
        tools: &[],
        max_tokens: config.llm.max_tokens,
    };

    match provider.chat(request).await {
        Ok(response) => {
            if let Some(reply) = response.text.as_deref() {
                println!("{reply}");
            }
            tracing::debug!(stop = ?response.stop, usage = ?response.usage, "lượt gọi xong");
            messages.push(Message::from_response(&response));
        }
        Err(err) => eprintln!("lỗi provider: {err}"),
    }
}

/// `/exit` và `/quit` kết thúc phiên (M8 sẽ có bộ slash command đầy đủ ở lõi).
fn is_exit_command(text: &str) -> bool {
    matches!(text, "/exit" | "/quit")
}

/// Vòng REPL tương tác (rustyline).
async fn run_interactive(config: &Config, provider: &Arc<dyn LlmProvider>) -> Result<()> {
    let mut editor = DefaultEditor::new().context("không khởi tạo được terminal")?;
    let mut messages: Vec<Message> = Vec::new();

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

        respond(config, provider, &mut messages, &text).await;
    }

    Ok(())
}

/// Chế độ không có TTY: đọc từng dòng từ stdin cho tới EOF.
async fn run_piped(config: &Config, provider: &Arc<dyn LlmProvider>) -> Result<()> {
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let mut messages: Vec<Message> = Vec::new();

    while let Some(line) = lines.next_line().await.context("đọc stdin thất bại")? {
        let text = line.trim().to_string();
        if text.is_empty() {
            continue;
        }
        if is_exit_command(&text) {
            break;
        }
        respond(config, provider, &mut messages, &text).await;
    }

    Ok(())
}
