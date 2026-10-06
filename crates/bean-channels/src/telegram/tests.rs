//! Test cho adapter Telegram.
//!
//! Chay **khong can mang, khong can bot token** nho vao seam `TelegramTransport`
//! (xem `types.rs`). Do do cac quy tac an toan cua kenh — allowlist user, chan callback
//! nguoi la, cat tin an toan — deu kiem chung duoc ma khong cham Telegram that.
//!
//! # Vì sao tach ra file riêng
//!
//! 777 dong test trong file adapter lam file "nho manh nhin khong ro" va de them mot
//! case se lam file lon them. Tach ra day giu file san pham chi con code chay that.
//!
//! Test duoc phep `unwrap` — nguyen tac "khong panic trong code san pham"
//! (agents.md muc 0.8) khong ap dung cho test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::Result;
use bean_core::{Channel, Decision, Router};

use super::targets::{CallbackAction, ParsedCallback, parse_callback};
use bean_types::config::TelegramConfig;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::*;

use async_trait::async_trait;
use bean_core::MemoryStore;
use bean_core::router::{RouterDeps, RouterOptions};
use bean_llm::FakeProvider;
use bean_security::CapWorkspace;
use bean_skills::{NewSkillDraft, SkillDraftKind};
use bean_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use bean_types::{Config, LlmResponse, Outbound, OutboundAction, Risk, ToolCall, ToolSpec};
use tokio::time::{Duration, timeout};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Text {
        chat_id: i64,
        text: String,
    },
    Confirmation {
        chat_id: i64,
        text: String,
        keyboard: TelegramKeyboard,
        message_id: i32,
    },
    Typing(i64),
    Edit {
        chat_id: i64,
        message_id: i32,
        text: String,
        remove_keyboard: bool,
    },
    Callback {
        id: String,
        text: Option<String>,
    },
}

#[derive(Default)]
struct MockTransport {
    updates: AsyncMutex<VecDeque<TelegramResult<Option<TelegramUpdate>>>>,
    commands: AsyncMutex<Vec<Command>>,
    next_message_id: AtomicUsize,
    prepared: AtomicUsize,
}

impl MockTransport {
    fn new() -> Self {
        Self::default()
    }

    async fn push_update(&self, update: TelegramUpdate) {
        self.updates.lock().await.push_back(Ok(Some(update)));
    }

    async fn push_error(&self, error: TelegramError) {
        self.updates.lock().await.push_back(Err(error));
    }

    async fn commands(&self) -> Vec<Command> {
        self.commands.lock().await.clone()
    }
}

#[async_trait]
impl TelegramTransport for MockTransport {
    async fn prepare(&self) -> TelegramResult<()> {
        self.prepared.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn next_update(&self) -> TelegramResult<Option<TelegramUpdate>> {
        match self.updates.lock().await.pop_front() {
            Some(result) => result,
            None => Ok(None),
        }
    }

    async fn send_text(&self, chat_id: i64, text: &str) -> TelegramResult<i32> {
        self.commands.lock().await.push(Command::Text {
            chat_id,
            text: text.to_owned(),
        });
        Ok(self.next_message_id.fetch_add(1, Ordering::SeqCst) as i32 + 100)
    }

    async fn send_confirmation(
        &self,
        chat_id: i64,
        text: &str,
        keyboard: TelegramKeyboard,
    ) -> TelegramResult<i32> {
        self.commands.lock().await.push(Command::Confirmation {
            chat_id,
            text: text.to_owned(),
            keyboard,
            message_id: self.next_message_id.fetch_add(1, Ordering::SeqCst) as i32 + 100,
        });
        Ok(self.next_message_id.load(Ordering::SeqCst) as i32 + 99)
    }

    async fn send_typing(&self, chat_id: i64) -> TelegramResult<()> {
        self.commands.lock().await.push(Command::Typing(chat_id));
        Ok(())
    }

    async fn edit_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        remove_keyboard: bool,
    ) -> TelegramResult<()> {
        self.commands.lock().await.push(Command::Edit {
            chat_id,
            message_id,
            text: text.to_owned(),
            remove_keyboard,
        });
        Ok(())
    }

    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> TelegramResult<()> {
        self.commands.lock().await.push(Command::Callback {
            id: callback_id.to_owned(),
            text: text.map(str::to_owned),
        });
        Ok(())
    }
}

struct RiskTool {
    risk: Risk,
    calls: AtomicUsize,
}

#[async_trait]
impl Tool for RiskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "risk_tool",
            "test risk",
            serde_json::json!({"type": "object"}),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        self.risk
    }

    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("done".into())
    }
}

fn telegram_config() -> TelegramConfig {
    TelegramConfig {
        enabled: true,
        token_env: "TEST_TELEGRAM_TOKEN".into(),
        allowed_user_ids: vec![42],
        rate_limit_per_minute: 20,
    }
}

async fn make_router(
    responses: Vec<LlmResponse>,
    tools: Vec<Arc<dyn Tool>>,
    options: RouterOptions,
) -> (tempfile::TempDir, Arc<Router>) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = Arc::new(CapWorkspace::open(temp.path().to_path_buf()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    for tool in tools {
        registry.register(tool).unwrap();
    }
    let mut config = Config::default();
    config.agent.allowed_users = vec!["telegram:42".into()];
    config.agent.max_steps = 4;
    let router = Arc::new(Router::with_options(
        RouterDeps {
            config,
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(registry),
            llm: Arc::new(FakeProvider::new(responses)),
            audit: None,
            skills_index: String::new(),
            skills: None,
        },
        options,
    ));
    (temp, router)
}

fn message_update(update_id: u32, user_id: i64, text: &str) -> TelegramUpdate {
    TelegramUpdate {
        update_id,
        kind: TelegramUpdateKind::Message(TelegramMessage {
            chat_id: 100,
            message_id: 1,
            user_id,
            text: text.to_owned(),
        }),
    }
}

fn callback_update(update_id: u32, user_id: i64, data: &str, message_id: i32) -> TelegramUpdate {
    TelegramUpdate {
        update_id,
        kind: TelegramUpdateKind::Callback(TelegramCallback {
            callback_id: format!("callback-{update_id}"),
            user_id,
            chat_id: Some(100),
            message_id: Some(message_id),
            data: data.to_owned(),
        }),
    }
}

async fn wait_command(transport: &MockTransport, predicate: impl Fn(&Command) -> bool) -> Command {
    let result = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(command) = transport.commands().await.into_iter().find(&predicate) {
                return command;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    match result {
        Ok(command) => command,
        Err(_) => panic!(
            "phải nhận Telegram command; commands={:?}",
            transport.commands().await
        ),
    }
}

fn spawn_channel(
    channel: Arc<TelegramChannel>,
    router: Arc<Router>,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(async move { channel.run(router, shutdown).await })
}

fn confirmation_buttons(command: &Command) -> Vec<TelegramButton> {
    match command {
        Command::Confirmation { keyboard, .. } => {
            keyboard.rows.first().cloned().unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

fn confirmation_message_id(command: &Command) -> i32 {
    match command {
        Command::Confirmation { message_id, .. } => *message_id,
        _ => 0,
    }
}

#[test]
fn split_text_preserves_utf8_and_utf16_limit() {
    let text = format!("{}{}", "Việt Nam 🦀 ".repeat(1_000), "😀".repeat(10));
    let parts = split_text(&text, 4_096);
    assert!(
        parts
            .iter()
            .all(|part| part.encode_utf16().count() <= 4_096)
    );
    assert_eq!(parts.concat(), text);
}

#[test]
fn split_text_handles_limit_smaller_than_one_emoji() {
    let text = "a😀b";
    let parts = split_text(text, 1);
    assert_eq!(parts.concat(), text);
    assert_eq!(parts, vec!["a", "😀", "b"]);
}

#[test]
fn split_text_cuts_before_scalar_that_exceeds_utf16_limit() {
    let text = "😀😀😀";
    let parts = split_text(text, 3);
    assert_eq!(parts.concat(), text);
    assert!(parts.iter().all(|part| part.encode_utf16().count() <= 3));
    assert_eq!(parts, vec!["😀", "😀", "😀"]);
}

#[test]
fn split_text_prefers_safe_boundaries() {
    let text = "một hai ba bốn năm";
    assert_eq!(split_text(text, 8), vec!["một hai ", "ba bốn ", "năm"]);
}

#[test]
fn rate_limit_is_per_chat_and_sliding() {
    let mut limiter = ChatRateLimiter::new(2);
    let start = Instant::now();
    assert!(limiter.allow(1, start));
    assert!(limiter.allow(1, start + Duration::from_secs(1)));
    assert!(!limiter.allow(1, start + Duration::from_secs(2)));
    assert!(limiter.allow(2, start + Duration::from_secs(2)));
    assert!(limiter.allow(1, start + Duration::from_secs(61)));
}

#[test]
fn update_dedup_is_bounded() {
    let mut dedup = UpdateDedup::new(2);
    assert!(dedup.accept(1));
    assert!(!dedup.accept(1));
    assert!(dedup.accept(2));
    assert!(dedup.accept(3));
    assert!(dedup.accept(1));
}

#[test]
fn callback_data_is_short_and_minimal() {
    let confirm_id = format!("confirm_{}", "a".repeat(32));
    let parsed = parse_callback(&format!("a:{confirm_id}"));
    assert_eq!(
        parsed,
        Some(ParsedCallback {
            action: CallbackAction::Confirm(Decision::Allow),
            id: confirm_id.clone(),
        })
    );
    assert_eq!(format!("a:{confirm_id}").len(), 42);
    assert_eq!(parse_callback("a:secret:extra"), None);
}

#[tokio::test]
async fn allowlist_blocks_unknown_user_without_reply() {
    let transport = Arc::new(MockTransport::new());
    transport
        .push_update(message_update(1, 999, "xin chào"))
        .await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let (_temp, router) = make_router(
        vec![LlmResponse::text_only("không được gửi")],
        vec![],
        RouterOptions::default(),
    )
    .await;
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router, shutdown.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;
    shutdown.cancel();
    task.await.unwrap().unwrap();
    assert!(transport.commands().await.is_empty());
}

#[tokio::test]
async fn dedup_does_not_submit_same_update_twice() {
    let transport = Arc::new(MockTransport::new());
    let update = message_update(7, 42, "chỉ một lần");
    transport.push_update(update.clone()).await;
    transport.push_update(update).await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let (_temp, router) = make_router(
        vec![LlmResponse::text_only("xong")],
        vec![],
        RouterOptions::default(),
    )
    .await;
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router, shutdown.clone());
    wait_command(
        &transport,
        |command| matches!(command, Command::Text { text, .. } if text == "xong"),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    shutdown.cancel();
    task.await.unwrap().unwrap();
    let finals = transport
        .commands()
        .await
        .into_iter()
        .filter(|command| matches!(command, Command::Text { text, .. } if text == "xong"))
        .count();
    assert_eq!(finals, 1);
}

#[tokio::test]
async fn rate_limited_message_is_not_submitted() {
    let mut config = telegram_config();
    config.rate_limit_per_minute = 1;
    let transport = Arc::new(MockTransport::new());
    transport.push_update(message_update(1, 42, "một")).await;
    transport.push_update(message_update(2, 42, "hai")).await;
    let channel = Arc::new(TelegramChannel::with_transport(transport.clone(), config));
    let (_temp, router) = make_router(
        vec![
            LlmResponse::text_only("trả lời một"),
            LlmResponse::text_only("trả lời hai"),
        ],
        vec![],
        RouterOptions::default(),
    )
    .await;
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router, shutdown.clone());
    wait_command(
        &transport,
        |command| matches!(command, Command::Text { text, .. } if text == "trả lời một"),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    shutdown.cancel();
    task.await.unwrap().unwrap();
    assert!(
        !transport
            .commands()
            .await
            .iter()
            .any(|command| matches!(command, Command::Text { text, .. } if text == "trả lời hai"))
    );
}

#[tokio::test]
async fn confirm_button_resolves_through_router() {
    let tool = Arc::new(RiskTool {
        risk: Risk::Confirm,
        calls: AtomicUsize::new(0),
    });
    let responses = vec![
        LlmResponse::with_tool_calls(vec![ToolCall::new(
            "c1",
            "risk_tool",
            serde_json::json!({}),
        )]),
        LlmResponse::text_only("đã xác nhận"),
    ];
    let (_temp, router) =
        make_router(responses, vec![tool.clone()], RouterOptions::default()).await;
    let transport = Arc::new(MockTransport::new());
    transport
        .push_update(message_update(1, 42, "chạy tool"))
        .await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router.clone(), shutdown.clone());
    let confirmation = wait_command(&transport, |command| {
        matches!(command, Command::Confirmation { .. })
    })
    .await;
    let buttons = confirmation_buttons(&confirmation);
    assert_eq!(buttons.len(), 3);
    assert!(
        buttons
            .iter()
            .all(|button| button.callback_data.len() <= 64)
    );
    assert!(buttons[0].callback_data.starts_with('a'));
    transport
        .push_update(callback_update(
            2,
            42,
            &buttons[0].callback_data,
            confirmation_message_id(&confirmation),
        ))
        .await;
    wait_command(
        &transport,
        |command| matches!(command, Command::Text { text, .. } if text == "đã xác nhận"),
    )
    .await;
    shutdown.cancel();
    task.await.unwrap().unwrap();
    assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dangerous_confirmation_has_no_session_button() {
    let tool = Arc::new(RiskTool {
        risk: Risk::Dangerous,
        calls: AtomicUsize::new(0),
    });
    let (_temp, router) = make_router(
        vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
            "c1",
            "risk_tool",
            serde_json::json!({}),
        )])],
        vec![tool],
        RouterOptions {
            confirm_timeout: Duration::from_millis(50),
            ..RouterOptions::default()
        },
    )
    .await;
    let transport = Arc::new(MockTransport::new());
    transport
        .push_update(message_update(1, 42, "nguy hiểm"))
        .await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel, router, shutdown.clone());
    let confirmation = wait_command(&transport, |command| {
        matches!(command, Command::Confirmation { .. })
    })
    .await;
    let buttons = confirmation_buttons(&confirmation);
    assert_eq!(buttons.len(), 2);
    assert!(
        buttons
            .iter()
            .all(|button| button.text != "Cho phép trong phiên")
    );
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn callback_from_unknown_user_does_not_resolve_confirm() {
    let tool = Arc::new(RiskTool {
        risk: Risk::Confirm,
        calls: AtomicUsize::new(0),
    });
    let (_temp, router) = make_router(
        vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
            "c1",
            "risk_tool",
            serde_json::json!({}),
        )])],
        vec![tool.clone()],
        RouterOptions {
            confirm_timeout: Duration::from_secs(30),
            ..RouterOptions::default()
        },
    )
    .await;
    let transport = Arc::new(MockTransport::new());
    transport
        .push_update(message_update(1, 42, "chạy tool"))
        .await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router.clone(), shutdown.clone());
    let confirmation = wait_command(&transport, |command| {
        matches!(command, Command::Confirmation { .. })
    })
    .await;
    let data = confirmation_buttons(&confirmation)[0].callback_data.clone();
    transport
        .push_update(callback_update(
            2,
            999,
            &data,
            confirmation_message_id(&confirmation),
        ))
        .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !transport
            .commands()
            .await
            .iter()
            .any(|command| matches!(command, Command::Callback { .. }))
    );
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert_eq!(router.snapshot().pending_confirms.len(), 1);
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn expired_confirm_is_edited_to_expired_text() {
    let tool = Arc::new(RiskTool {
        risk: Risk::Confirm,
        calls: AtomicUsize::new(0),
    });
    let (_temp, router) = make_router(
        vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
            "c1",
            "risk_tool",
            serde_json::json!({}),
        )])],
        vec![tool.clone()],
        RouterOptions {
            confirm_timeout: Duration::from_millis(30),
            ..RouterOptions::default()
        },
    )
    .await;
    let transport = Arc::new(MockTransport::new());
    transport
        .push_update(message_update(1, 42, "chạy tool"))
        .await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router, shutdown.clone());
    wait_command(&transport, |command| {
        matches!(command, Command::Edit { text, remove_keyboard: true, .. } if text == "Hết hạn")
    })
    .await;
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn polling_retries_network_error_and_reports_conflict() {
    let transport = Arc::new(MockTransport::new());
    transport.push_error(TelegramError::Network).await;
    transport
        .push_update(message_update(1, 42, "sau reconnect"))
        .await;
    let channel = Arc::new(TelegramChannel::with_transport(
        transport.clone(),
        telegram_config(),
    ));
    let (_temp, router) = make_router(
        vec![LlmResponse::text_only("đã nhận")],
        vec![],
        RouterOptions::default(),
    )
    .await;
    let shutdown = CancellationToken::new();
    let task = spawn_channel(channel.clone(), router, shutdown.clone());
    wait_command(
        &transport,
        |command| matches!(command, Command::Text { text, .. } if text == "đã nhận"),
    )
    .await;
    shutdown.cancel();
    task.await.unwrap().unwrap();
    assert_eq!(transport.prepared.load(Ordering::SeqCst), 1);

    let conflict_transport = Arc::new(MockTransport::new());
    conflict_transport.push_error(TelegramError::Conflict).await;
    let conflict_channel = Arc::new(TelegramChannel::with_transport(
        conflict_transport,
        telegram_config(),
    ));
    let (_temp, router) = make_router(
        vec![LlmResponse::text_only("x")],
        vec![],
        RouterOptions::default(),
    )
    .await;
    let error = conflict_channel
        .run(router, CancellationToken::new())
        .await
        .expect_err("409 phải là fatal");
    assert!(error.to_string().contains("409"));
}
#[tokio::test]
async fn skill_draft_inline_button_approves_for_exact_allowed_user() {
    let temp = tempfile::tempdir().unwrap();
    let skills_root = temp.path().join("skills");
    let catalog = bean_skills::SkillCatalog::load_with_paths(
        std::slice::from_ref(&skills_root),
        skills_root.clone(),
        skills_root.join("_drafts"),
    );
    let draft = catalog
        .create_draft(NewSkillDraft {
            name: "release-checklist".into(),
            kind: SkillDraftKind::New,
            description: "Dùng trước khi phát hành.".into(),
            body: "# Steps\n1. Chạy test.".into(),
            reason: "Quy trình lặp lại.".into(),
            source_session_id: 1,
            source_channel: "telegram".into(),
            source_chat_id: "100".into(),
            created_at: "2026-09-25T10:00:00Z".into(),
        })
        .unwrap();
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
    let mut config = Config::default();
    config.agent.allowed_users = vec!["telegram:42".into()];
    let router = Arc::new(Router::new(RouterDeps {
        config,
        store: Arc::new(MemoryStore::new()),
        registry: Arc::new(ToolRegistry::with_workspace(workspace)),
        llm: Arc::new(FakeProvider::echo()),
        audit: None,
        skills_index: catalog.index(),
        skills: Some(catalog.clone()),
    }));
    let transport = Arc::new(MockTransport::new());
    let mut telegram_config = telegram_config();
    telegram_config.allowed_user_ids.push(43);
    let channel = TelegramChannel::with_transport(transport.clone(), telegram_config);
    channel
        .send(
            "100",
            Outbound {
                session_id: bean_types::SessionId::new(1),
                message_id: 1,
                text: format!("Đề xuất `{}`", draft.name),
                kind: bean_types::OutboundKind::Notification,
                action: Some(OutboundAction::SkillDraft {
                    id: draft.id.clone(),
                    name: draft.name.clone(),
                    actor: "telegram:42".into(),
                }),
            },
        )
        .await
        .unwrap();
    let command = wait_command(&transport, |command| {
        matches!(command, Command::Confirmation { .. })
    })
    .await;
    let message_id = confirmation_message_id(&command);
    let buttons = confirmation_buttons(&command);
    assert_eq!(buttons[0].callback_data, format!("skill:a:{}", draft.id));
    assert_eq!(buttons[1].callback_data, format!("skill:r:{}", draft.id));

    let wrong_user = callback_update(98, 43, &buttons[0].callback_data, message_id);
    channel
        .process_update(&router, wrong_user, &CancellationToken::new())
        .await;
    wait_command(&transport, |command| {
        matches!(command, Command::Callback { text: Some(text), .. } if text == "Yêu cầu không hợp lệ")
    })
    .await;
    assert!(catalog.get("release-checklist").is_err());

    let update = callback_update(99, 42, &buttons[0].callback_data, message_id);
    channel
        .process_update(&router, update, &CancellationToken::new())
        .await;
    wait_command(&transport, |command| {
        matches!(command, Command::Edit { text, remove_keyboard: true, .. } if text == "Đã duyệt skill")
    })
    .await;
    assert!(catalog.get("release-checklist").is_ok());
}
