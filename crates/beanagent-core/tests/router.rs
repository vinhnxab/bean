//! Regression M8: Router queue, confirm, cancel, slash command và outbox.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use beanagent_core::router::{Channel, Router, RouterDeps, RouterOptions};
use beanagent_core::store::{MemoryStore, Store};
use beanagent_core::{Decision, Incoming, RouterError};
use beanagent_llm::{ChatRequest, FakeProvider, LlmError, LlmProvider};
use beanagent_security::{AuditLog, CapWorkspace};
use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use beanagent_types::{
    Config, ConfirmOutcome, LlmResponse, Outbound, OutboundKind, Risk, RunEvent, RunId, SessionId,
    ToolCall, ToolSpec,
};
use tokio::sync::{Notify, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

fn config() -> Config {
    let mut config = Config::default();
    config.agent.max_steps = 6;
    config.security.tool_timeout_seconds = 5;
    config
}

async fn router_with(
    store: Arc<MemoryStore>,
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Arc<dyn Tool>>,
    options: RouterOptions,
) -> (tempfile::TempDir, Arc<Router>) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = Arc::new(CapWorkspace::open(temp.path().to_path_buf()).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    for tool in tools {
        registry.register(tool).unwrap();
    }
    let router = Arc::new(Router::with_options(
        RouterDeps {
            config: config(),
            store,
            registry: Arc::new(registry),
            llm: provider,
            audit: None,
            skills_index: "demo: Dùng cho test".into(),
        },
        options,
    ));
    (temp, router)
}

fn fake(text: &str) -> Arc<dyn LlmProvider> {
    Arc::new(FakeProvider::new(vec![LlmResponse::text_only(text)]))
}

fn tool_calls(calls: Vec<ToolCall>) -> LlmResponse {
    LlmResponse::with_tool_calls(calls)
}

fn probe_call(id: &str) -> ToolCall {
    ToolCall::new(id, "probe", serde_json::json!({}))
}

async fn next_event(receiver: &mut broadcast::Receiver<RunEvent>, run_id: &RunId) -> RunEvent {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), receiver.recv())
            .await
            .expect("phải có event trong 3s")
            .expect("broadcast phải còn mở");
        if event_uses_run(&event, run_id) {
            return event;
        }
    }
}

fn event_uses_run(event: &RunEvent, run_id: &RunId) -> bool {
    match event {
        RunEvent::Queued { run_id: id, .. }
        | RunEvent::Text { run_id: id, .. }
        | RunEvent::ToolStart { run_id: id, .. }
        | RunEvent::ToolEnd { run_id: id, .. }
        | RunEvent::ConfirmRequest { run_id: id, .. }
        | RunEvent::ConfirmResolved { run_id: id, .. }
        | RunEvent::Final { run_id: id, .. }
        | RunEvent::Error { run_id: id, .. } => id == run_id,
    }
}

async fn wait_final(receiver: &mut broadcast::Receiver<RunEvent>, run_id: &RunId) -> String {
    loop {
        match next_event(receiver, run_id).await {
            RunEvent::Final { text, .. } => return text,
            RunEvent::Error { code, message, .. } => {
                panic!("run lỗi {code}: {message}")
            }
            _ => {}
        }
    }
}

struct GateProvider {
    responses: Mutex<VecDeque<LlmResponse>>,
    started: Notify,
    gate: Semaphore,
    calls: AtomicUsize,
}

impl std::fmt::Debug for GateProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GateProvider")
            .field("calls", &self.calls)
            .finish()
    }
}

#[async_trait]
impl LlmProvider for GateProvider {
    async fn chat(&self, _request: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            self.started.notify_one();
            self.gate.acquire().await.unwrap().forget();
        }
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(LlmError::FakeScript("hết kịch bản".into()))
    }

    fn name(&self) -> &'static str {
        "gate"
    }
}

struct SlowProvider {
    delay: Duration,
    finished: Notify,
}

impl std::fmt::Debug for SlowProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowProvider").finish()
    }
}

#[async_trait]
impl LlmProvider for SlowProvider {
    async fn chat(&self, _request: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        tokio::time::sleep(self.delay).await;
        self.finished.notify_waiters();
        Ok(LlmResponse::text_only("xong"))
    }

    fn name(&self) -> &'static str {
        "slow"
    }
}

struct ModelProvider {
    seen: Mutex<Vec<String>>,
}

impl std::fmt::Debug for ModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelProvider").finish()
    }
}

#[async_trait]
impl LlmProvider for ModelProvider {
    async fn chat(&self, request: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.chat_with_model(request, "default").await
    }

    async fn chat_with_model(
        &self,
        _request: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmResponse, LlmError> {
        self.seen.lock().unwrap().push(model.to_string());
        Ok(LlmResponse::text_only(model))
    }

    fn name(&self) -> &'static str {
        "model"
    }
}

struct Probe {
    risk: Risk,
    delay: Duration,
    calls: AtomicUsize,
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe")
            .field("risk", &self.risk)
            .field("calls", &self.calls)
            .finish()
    }
}

#[async_trait]
impl Tool for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("probe", "tool test", serde_json::json!({"type": "object"}))
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        self.risk
    }

    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok("probe-ok".into())
    }
}

struct FlakyChannel {
    fail_first: bool,
    calls: AtomicUsize,
    sent: Mutex<Vec<Outbound>>,
}

#[async_trait]
impl Channel for FlakyChannel {
    fn name(&self) -> &'static str {
        "test"
    }

    async fn run(&self, _router: Arc<Router>, shutdown: CancellationToken) -> AnyResult<()> {
        shutdown.cancelled().await;
        Ok(())
    }

    async fn send(&self, _chat_id: &str, out: Outbound) -> AnyResult<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_first && call == 0 {
            anyhow::bail!("lỗi mạng giả lập");
        }
        self.sent.lock().unwrap().push(out);
        Ok(())
    }
}

#[tokio::test]
async fn queue_runs_one_at_a_time_and_marks_later_runs() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(GateProvider {
        responses: Mutex::new(VecDeque::from([
            LlmResponse::text_only("thứ nhất"),
            LlmResponse::text_only("thứ hai"),
        ])),
        started: Notify::new(),
        gate: Semaphore::new(0),
        calls: AtomicUsize::new(0),
    });
    let started = provider.started.notified();
    let (_temp, router) = router_with(
        store.clone(),
        provider.clone(),
        vec![],
        RouterOptions::default(),
    )
    .await;
    let mut events = router.events();

    let first = router
        .submit(Incoming::new("cli", "local", "cli:local", "một"))
        .await
        .unwrap();
    started.await;
    let second = router
        .submit(Incoming::new("cli", "local", "cli:local", "hai"))
        .await
        .unwrap();
    assert!(matches!(
        next_event(&mut events, &second).await,
        RunEvent::Queued { position: 1, .. }
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

    provider.gate.add_permits(1);
    assert_eq!(wait_final(&mut events, &first).await, "thứ nhất");
    assert_eq!(wait_final(&mut events, &second).await, "thứ hai");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn dropping_all_subscribers_does_not_cancel_run() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(SlowProvider {
        delay: Duration::from_millis(30),
        finished: Notify::new(),
    });
    let (_temp, router) =
        router_with(store.clone(), provider, vec![], RouterOptions::default()).await;
    let events = router.events();
    drop(events);
    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "chạy nền"))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        while router.active_run("cli", "local").is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("run không được tự treo");
    let session = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();
    let history = store.history(session, None, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].text.as_deref(), Some("xong"));
    let _ = run;
}

#[tokio::test]
async fn recv_event_recovers_after_lagged_subscriber() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(FakeProvider::new(vec![
        tool_calls(vec![probe_call("c1")]),
        LlmResponse::text_only("xong"),
    ]));
    let probe = Arc::new(Probe {
        risk: Risk::Safe,
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    });
    let options = RouterOptions {
        event_capacity: 1,
        ..RouterOptions::default()
    };
    let (_temp, router) = router_with(store, provider, vec![probe], options).await;
    let mut events = router.events();
    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "nhiều event"))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), async {
        while router.active_run("cli", "local").is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        router.recv_event(&mut events).await,
        Some(RunEvent::Final { text, .. }) if text == "xong"
    ));
    let _ = run;
}

#[tokio::test]
async fn confirm_timeout_becomes_deny_and_continues_history() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(FakeProvider::new(vec![
        tool_calls(vec![probe_call("c1")]),
        LlmResponse::text_only("đã xử lý"),
    ]));
    let probe = Arc::new(Probe {
        risk: Risk::Confirm,
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    });
    let options = RouterOptions {
        confirm_timeout: Duration::from_millis(30),
        ..RouterOptions::default()
    };
    let (_temp, router) = router_with(store.clone(), provider, vec![probe.clone()], options).await;
    let mut events = router.events();
    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "cần xác nhận"))
        .await
        .unwrap();
    let confirm_id = loop {
        if let RunEvent::ConfirmRequest { confirm_id, .. } = next_event(&mut events, &run).await {
            break confirm_id;
        }
    };
    assert!(matches!(
        next_event(&mut events, &run).await,
        RunEvent::ConfirmResolved {
            outcome: ConfirmOutcome::Expired,
            ..
        }
    ));
    assert_eq!(wait_final(&mut events, &run).await, "đã xử lý");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 0);

    let session = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();
    let history = store.history(session, None, 0).await.unwrap();
    let tool = history
        .iter()
        .find(|msg| msg.role == beanagent_types::Role::Tool)
        .unwrap();
    assert!(tool.is_error);
    assert!(tool.text.as_deref().unwrap().contains("Hết thời gian"));
    let _ = confirm_id;
}

#[tokio::test]
async fn first_valid_confirm_wins_and_later_response_is_rejected() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(FakeProvider::new(vec![
        tool_calls(vec![probe_call("c1")]),
        LlmResponse::text_only("xong"),
    ]));
    let probe = Arc::new(Probe {
        risk: Risk::Confirm,
        delay: Duration::ZERO,
        calls: AtomicUsize::new(0),
    });
    let (_temp, router) = router_with(
        store,
        provider,
        vec![probe.clone()],
        RouterOptions::default(),
    )
    .await;
    let mut events = router.events();
    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "duyệt"))
        .await
        .unwrap();
    let confirm_id = loop {
        if let RunEvent::ConfirmRequest { confirm_id, .. } = next_event(&mut events, &run).await {
            break confirm_id;
        }
    };
    assert!(matches!(
        router
            .resolve_confirm(confirm_id.as_str(), Decision::Allow, "cli:khác")
            .await,
        Err(RouterError::ConfirmForbidden)
    ));
    router
        .resolve_confirm(confirm_id.as_str(), Decision::Allow, "cli:local")
        .await
        .unwrap();
    assert!(matches!(
        router
            .resolve_confirm(confirm_id.as_str(), Decision::Deny, "cli:local")
            .await,
        Err(RouterError::ConfirmNotFound)
    ));
    assert_eq!(wait_final(&mut events, &run).await, "xong");
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn confirm_actor_is_written_to_audit_log() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let audit = Arc::new(AuditLog::open(&temp.path().join("audit")).unwrap());
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    registry
        .register(Arc::new(Probe {
            risk: Risk::Confirm,
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
        }))
        .unwrap();
    let provider = Arc::new(FakeProvider::new(vec![
        tool_calls(vec![probe_call("c1")]),
        LlmResponse::text_only("xong"),
    ]));
    let router = Arc::new(Router::new(RouterDeps {
        config: config(),
        store,
        registry: Arc::new(registry),
        llm: provider,
        audit: Some(audit),
        skills_index: String::new(),
    }));
    let mut events = router.events();
    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "audit"))
        .await
        .unwrap();
    let confirm_id = loop {
        if let RunEvent::ConfirmRequest { confirm_id, .. } = next_event(&mut events, &run).await {
            break confirm_id;
        }
    };
    router
        .resolve_confirm(confirm_id.as_str(), Decision::Allow, "cli:local")
        .await
        .unwrap();
    wait_final(&mut events, &run).await;

    let content = std::fs::read_to_string(temp.path().join("audit/audit.jsonl")).unwrap();
    let entry: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(entry["decided_by"], "cli:local");
    assert_eq!(entry["decision"], "allow");
}

#[tokio::test]
async fn cancel_keeps_every_tool_call_paired() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(FakeProvider::new(vec![tool_calls(vec![
        probe_call("c1"),
        probe_call("c2"),
    ])]));
    let probe = Arc::new(Probe {
        risk: Risk::Safe,
        delay: Duration::from_secs(30),
        calls: AtomicUsize::new(0),
    });
    let (_temp, router) = router_with(
        store.clone(),
        provider,
        vec![probe.clone()],
        RouterOptions::default(),
    )
    .await;
    let mut events = router.events();
    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "làm hai tool"))
        .await
        .unwrap();
    loop {
        if let RunEvent::ToolStart { id, .. } = next_event(&mut events, &run).await
            && id == "c1"
        {
            break;
        }
    }
    router.cancel("cli", "local").await;
    loop {
        if let RunEvent::Error { code, .. } = next_event(&mut events, &run).await {
            assert_eq!(code, "cancelled");
            break;
        }
    }
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);

    let session = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();
    let history = store.history(session, None, 0).await.unwrap();
    let assistant = history
        .iter()
        .find(|msg| msg.role == beanagent_types::Role::Assistant)
        .unwrap();
    assert_eq!(assistant.tool_calls.len(), 2);
    let results: Vec<_> = history
        .iter()
        .filter(|msg| msg.role == beanagent_types::Role::Tool)
        .collect();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|msg| msg.is_error));
    assert!(
        results
            .iter()
            .all(|msg| msg.text.as_deref() == Some("[bị người dùng huỷ]"))
    );
    assert!(beanagent_memory::check_no_orphan_result(&history, 0));
}

#[tokio::test]
async fn new_archives_old_session_and_creates_new_one() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(FakeProvider::new(vec![LlmResponse::text_only("tin cũ")]));
    let (_temp, router) =
        router_with(store.clone(), provider, vec![], RouterOptions::default()).await;
    let mut events = router.events();
    let first_run = router
        .submit(Incoming::new("cli", "local", "cli:local", "trước /new"))
        .await
        .unwrap();
    wait_final(&mut events, &first_run).await;
    let old = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();

    let command_run = router
        .submit(Incoming::new("cli", "local", "cli:local", "/new"))
        .await
        .unwrap();
    assert!(
        wait_final(&mut events, &command_run)
            .await
            .contains("phiên mới")
    );
    let new = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();
    assert_ne!(old, new);
    assert!(store.session_info(old).await.unwrap().unwrap().archived);
    assert!(!store.session_info(new).await.unwrap().unwrap().archived);
}

#[tokio::test]
async fn explicit_session_id_from_another_user_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let owned = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();
    let mut config = config();
    config.agent.allowed_users = vec!["cli:local".into(), "cli:other".into()];
    let router = Router::new(RouterDeps {
        config,
        store,
        registry: Arc::new(ToolRegistry::with_workspace(Arc::new(
            CapWorkspace::open(temp.path().to_path_buf()).unwrap(),
        ))),
        llm: fake("không được gọi"),
        audit: None,
        skills_index: String::new(),
    });
    let result = router
        .submit(Incoming::new("cli", "local", "cli:other", "cướp session").with_session(owned))
        .await;
    assert!(matches!(result, Err(RouterError::InvalidSession)));
}

#[tokio::test]
async fn slash_commands_do_not_call_llm() {
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(FakeProvider::new(vec![LlmResponse::text_only(
        "không được gọi",
    )]));
    let (_temp, router) =
        router_with(store, provider.clone(), vec![], RouterOptions::default()).await;
    let mut events = router.events();
    for (command, expected) in [
        ("/skills", "demo:"),
        ("/tasks", "Chưa có"),
        ("/approve skill-1", "M15"),
        ("/reject skill-1", "M15"),
    ] {
        let run = router
            .submit(Incoming::new("cli", "local", "cli:local", command))
            .await
            .unwrap();
        assert!(wait_final(&mut events, &run).await.contains(expected));
    }
    assert_eq!(provider.remaining(), 1);
}

#[tokio::test]
async fn user_outside_allowlist_is_rejected_before_session_creation() {
    let store = Arc::new(MemoryStore::new());
    let (_temp, router) = router_with(
        store.clone(),
        fake("không được gọi"),
        vec![],
        RouterOptions::default(),
    )
    .await;
    assert!(
        store
            .session_info(SessionId::new(1))
            .await
            .unwrap()
            .is_none()
    );
    let result = router
        .submit(Incoming::new("cli", "local", "cli:kẻ_lạ", "xin chào"))
        .await;
    assert!(matches!(result, Err(RouterError::Forbidden(user)) if user == "cli:kẻ_lạ"));
    assert!(
        store
            .session_info(SessionId::new(1))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn model_command_applies_override_to_next_run_only() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(MemoryStore::new());
    let provider = Arc::new(ModelProvider {
        seen: Mutex::new(Vec::new()),
    });
    let workspace = Arc::new(CapWorkspace::open(temp.path().to_path_buf()).unwrap());
    let mut config = config();
    config.llm.model = "model-a".into();
    config.llm.allowed_models = vec!["model-a".into(), "model-b".into()];
    let router = Arc::new(Router::new(RouterDeps {
        config,
        store,
        registry: Arc::new(ToolRegistry::with_workspace(workspace)),
        llm: provider.clone(),
        audit: None,
        skills_index: String::new(),
    }));
    let mut events = router.events();

    let command = router
        .submit(Incoming::new("cli", "local", "cli:local", "/model model-b"))
        .await
        .unwrap();
    assert!(wait_final(&mut events, &command).await.contains("model-b"));
    assert!(provider.seen.lock().unwrap().is_empty());

    let run = router
        .submit(Incoming::new("cli", "local", "cli:local", "chạy"))
        .await
        .unwrap();
    assert_eq!(wait_final(&mut events, &run).await, "model-b");
    assert_eq!(provider.seen.lock().unwrap().as_slice(), &["model-b"]);

    let denied = router
        .submit(Incoming::new("cli", "local", "cli:local", "/model model-c"))
        .await
        .unwrap();
    assert!(matches!(
        next_event(&mut events, &denied).await,
        RunEvent::Error { code, .. } if code == "model_not_allowed"
    ));
}

#[tokio::test]
async fn outbox_retries_with_backoff_then_succeeds() {
    let store = Arc::new(MemoryStore::new());
    let options = RouterOptions {
        outbox_base_delay: Duration::from_millis(20),
        outbox_max_delay: Duration::from_millis(100),
        ..RouterOptions::default()
    };
    let (_temp, router) = router_with(store.clone(), fake("x"), vec![], options).await;
    let out = Outbound {
        session_id: SessionId::new(1),
        message_id: 9,
        text: "tin cần gửi".into(),
        kind: OutboundKind::Notification,
    };
    router.notify("test", "chat-1", out.clone()).await.unwrap();
    let channel = Arc::new(FlakyChannel {
        fail_first: true,
        calls: AtomicUsize::new(0),
        sent: Mutex::new(Vec::new()),
    });
    router.register_channel(channel.clone()).unwrap();

    assert_eq!(router.process_outbox_once().await.unwrap(), 1);
    assert_eq!(channel.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        router.process_outbox_once().await.unwrap(),
        0,
        "backoff chưa đến hạn thì không được gửi lại"
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(router.process_outbox_once().await.unwrap(), 1);
    assert_eq!(channel.calls.load(Ordering::SeqCst), 2);
    assert_eq!(channel.sent.lock().unwrap().as_slice(), &[out]);
    assert!(
        store
            .due_outbox("2999-01-01T00:00:00.000000Z", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
