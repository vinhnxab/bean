//! Regression M13: fixed clock, timezone cron, due-once, cancel, background policy, outbox.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bean_core::store::MemoryStore;
use bean_core::{
    Channel, FixedClock, Router, RouterDeps, RouterOptions, Scheduler, next_run_after,
};
use bean_llm::FakeProvider;
use bean_memory::{NewScheduledTask, Store};
use bean_security::CapWorkspace;
use bean_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use bean_types::{Config, LlmResponse, Outbound, OutboundKind, Risk, ToolCall, ToolSpec};
use chrono::{DateTime, TimeZone, Utc};
use tokio_util::sync::CancellationToken;

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("thời điểm test hợp lệ")
}

fn config() -> Config {
    let mut config = Config::default();
    config.agent.max_steps = 4;
    config.agent.timezone = "Asia/Ho_Chi_Minh".into();
    config.agent.allowed_users.push("test:user".into());
    config
}

#[derive(Debug)]
struct ConfirmTool {
    calls: AtomicUsize,
}

#[async_trait]
impl Tool for ConfirmTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "confirm_tool",
            "tool cần xác nhận cho test scheduler",
            serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false}),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Confirm
    }

    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok("đã chạy".into())
    }
}

#[derive(Debug)]
struct CaptureChannel {
    fail_first: bool,
    calls: AtomicUsize,
    sent: Mutex<Vec<Outbound>>,
}

#[async_trait]
impl Channel for CaptureChannel {
    fn name(&self) -> &'static str {
        "test"
    }

    async fn run(&self, _router: Arc<Router>, shutdown: CancellationToken) -> AnyResult<()> {
        shutdown.cancelled().await;
        Ok(())
    }

    async fn send(&self, _chat_id: &str, out: Outbound) -> AnyResult<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_first && call < 2 {
            anyhow::bail!("lỗi gửi giả lập");
        }
        self.sent.lock().unwrap().push(out);
        Ok(())
    }
}

async fn fixture(
    responses: Vec<LlmResponse>,
    tool: Option<Arc<ConfirmTool>>,
    fail_first: bool,
    now: DateTime<Utc>,
) -> (
    tempfile::TempDir,
    Arc<MemoryStore>,
    Arc<Router>,
    Arc<FixedClock>,
    Arc<CaptureChannel>,
) {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    if let Some(tool) = tool {
        registry.register(tool).unwrap();
    }
    let router = Arc::new(Router::with_options(
        RouterDeps {
            config: config(),
            store: store.clone(),
            registry: Arc::new(registry),
            llm: Arc::new(FakeProvider::new(responses)),
            audit: None,
            skills_index: String::new(),
            skills: None,
        },
        RouterOptions {
            outbox_base_delay: Duration::from_millis(1),
            outbox_max_delay: Duration::from_millis(2),
            ..RouterOptions::default()
        },
    ));
    let channel = Arc::new(CaptureChannel {
        fail_first,
        calls: AtomicUsize::new(0),
        sent: Mutex::new(Vec::new()),
    });
    router.register_channel(channel.clone()).unwrap();
    let clock = Arc::new(FixedClock::new(now));
    (temp, store, router, clock, channel)
}

fn task_input(
    session: bean_types::SessionId,
    now: DateTime<Utc>,
    allowed_tools: Vec<String>,
) -> NewScheduledTask {
    NewScheduledTask {
        cron: "0 7 * * *".into(),
        prompt: "tóm tắt việc cần làm".into(),
        session_id: Some(session),
        channel: "test".into(),
        chat_id: "chat".into(),
        allowed_tools,
        next_run: now.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        enabled: true,
    }
}

#[test]
fn cron_uses_asia_ho_chi_minh_and_returns_utc() {
    let now = at(2026, 9, 25, 23, 0);
    let next = next_run_after("0 7 * * *", "Asia/Ho_Chi_Minh", now).unwrap();
    assert_eq!(next, at(2026, 9, 26, 0, 0));
}

#[tokio::test]
async fn due_task_runs_exactly_once_and_uses_fake_clock() {
    let now = at(2026, 9, 26, 0, 0);
    let (_temp, store, router, clock, channel) =
        fixture(vec![LlmResponse::text_only("xong")], None, false, now).await;
    let session = store
        .ensure_session_for_user("test", "chat", "test:user", "")
        .await
        .unwrap();
    let created = store
        .create_task(task_input(session, now, vec![]))
        .await
        .unwrap();
    let scheduler = Scheduler::with_clock(
        store.clone(),
        router.clone(),
        "Asia/Ho_Chi_Minh",
        clock.clone(),
    )
    .unwrap();
    assert_eq!(scheduler.run_once().await.unwrap(), 1);
    assert_eq!(channel.calls.load(Ordering::SeqCst), 1);
    assert_eq!(channel.sent.lock().unwrap()[0].text, "xong");
    clock.set(now + chrono::Duration::hours(1));
    assert_eq!(scheduler.run_once().await.unwrap(), 0);
    let task = store
        .list_tasks()
        .await
        .unwrap()
        .into_iter()
        .find(|task| task.id == created.id)
        .unwrap();
    assert_eq!(task.last_status, "success");
}

#[tokio::test]
async fn cancelled_task_does_not_run() {
    let now = at(2026, 9, 26, 0, 0);
    let (_temp, store, router, clock, channel) = fixture(
        vec![LlmResponse::text_only("không được chạy")],
        None,
        false,
        now,
    )
    .await;
    let session = store
        .ensure_session_for_user("test", "chat", "test:user", "")
        .await
        .unwrap();
    let created = store
        .create_task(task_input(session, now, vec![]))
        .await
        .unwrap();
    assert!(
        store
            .delete_task_for_session(created.id, session)
            .await
            .unwrap()
    );
    let scheduler = Scheduler::with_clock(store, router, "Asia/Ho_Chi_Minh", clock).unwrap();
    assert_eq!(scheduler.run_once().await.unwrap(), 0);
    assert_eq!(channel.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn background_confirm_is_denied_unless_explicitly_allowed() {
    let now = at(2026, 9, 26, 0, 0);
    let tool = Arc::new(ConfirmTool {
        calls: AtomicUsize::new(0),
    });
    let call = ToolCall::new("c1", "confirm_tool", serde_json::json!({}));
    let (_temp, store, router, clock, _channel) = fixture(
        vec![
            LlmResponse::with_tool_calls(vec![call]),
            LlmResponse::text_only("đã xử lý nền"),
        ],
        Some(tool.clone()),
        false,
        now,
    )
    .await;
    let session = store
        .ensure_session_for_user("test", "chat", "test:user", "")
        .await
        .unwrap();
    store
        .create_task(task_input(session, now, vec![]))
        .await
        .unwrap();
    let scheduler =
        Scheduler::with_clock(store.clone(), router, "Asia/Ho_Chi_Minh", clock).unwrap();
    assert_eq!(scheduler.run_once().await.unwrap(), 1);
    assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
    assert!(
        store
            .history(session, None, 0)
            .await
            .unwrap()
            .iter()
            .any(|m| m.is_error)
    );

    let allowed_tool = Arc::new(ConfirmTool {
        calls: AtomicUsize::new(0),
    });
    let allowed_call = ToolCall::new("c2", "confirm_tool", serde_json::json!({}));
    let (_temp2, store2, router2, clock2, _channel2) = fixture(
        vec![
            LlmResponse::with_tool_calls(vec![allowed_call]),
            LlmResponse::text_only("đã chạy nền"),
        ],
        Some(allowed_tool.clone()),
        false,
        now,
    )
    .await;
    let session2 = store2
        .ensure_session_for_user("test", "chat", "test:user", "")
        .await
        .unwrap();
    store2
        .create_task(task_input(session2, now, vec!["confirm_tool".into()]))
        .await
        .unwrap();
    let scheduler2 = Scheduler::with_clock(store2, router2, "Asia/Ho_Chi_Minh", clock2).unwrap();
    assert_eq!(scheduler2.run_once().await.unwrap(), 1);
    assert_eq!(allowed_tool.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn outbox_keeps_failed_notification_and_delivers_on_retry() {
    let now = at(2026, 9, 26, 0, 0);
    let (_temp, store, router, _clock, channel) =
        fixture(vec![LlmResponse::text_only("tin bù")], None, true, now).await;
    let session = store
        .ensure_session_for_user("test", "chat", "test:user", "")
        .await
        .unwrap();
    let message_id = store
        .append(
            session,
            bean_types::Message::assistant(Some("tin bù".into()), vec![]),
        )
        .await
        .unwrap();
    let out = Outbound {
        session_id: session,
        message_id,
        text: "tin bù".into(),
        kind: OutboundKind::Notification,
        action: None,
    };
    router.notify("test", "chat", out.clone()).await.unwrap();
    assert_eq!(
        store
            .due_outbox("2999-01-01T00:00:00.000000Z", 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(router.process_outbox_once().await.unwrap(), 1);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(router.process_outbox_once().await.unwrap(), 1);
    assert_eq!(channel.calls.load(Ordering::SeqCst), 3);
    assert_eq!(channel.sent.lock().unwrap().as_slice(), &[out]);
    assert!(
        store
            .due_outbox("2999-01-01T00:00:00.000000Z", 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn one_hour_scheduler_smoke_with_fast_tick() {
    let now = at(2026, 9, 26, 0, 0);
    let responses = (0..60)
        .map(|_| LlmResponse::text_only("tick"))
        .collect::<Vec<_>>();
    let (_temp, store, router, clock, channel) = fixture(responses, None, false, now).await;
    let session = store
        .ensure_session_for_user("test", "chat", "test:user", "")
        .await
        .unwrap();
    let mut task = task_input(session, now, vec![]);
    task.cron = "* * * * *".into();
    store.create_task(task).await.unwrap();
    let scheduler = Scheduler::with_clock(store, router, "UTC", clock.clone())
        .unwrap()
        .with_tick(Duration::from_millis(1));
    for _ in 0..60 {
        assert_eq!(scheduler.run_once().await.unwrap(), 1);
        clock.advance(Duration::from_secs(60));
    }
    assert_eq!(channel.sent.lock().unwrap().len(), 60);
}
