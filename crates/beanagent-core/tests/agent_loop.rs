//! Test vòng lặp agent (M3) — mọi tình huống chạy với provider dựng sẵn, không mạng,
//! không tốn tiền (agents.md mục 20).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use beanagent_core::agent::AgentError;
use beanagent_core::run_io::{Decision, RunIo};
use beanagent_core::store::{MemoryStore, Store};
use beanagent_core::{RunTurnArgs, run_turn};
use beanagent_llm::{ChatRequest, LlmError, LlmProvider};
use beanagent_security::CapWorkspace;
use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use beanagent_types::{
    Config, LlmDelta, LlmResponse, LlmToolCallDelta, Risk, Role, RolePermissions, SessionId,
    StopReason, ToolCall, ToolSpec, Usage,
};
use tokio_util::sync::CancellationToken;

/// Provider dựng sẵn: trả lần lượt danh sách response; hết thì lỗi (không panic).
struct ScriptProvider {
    responses: Vec<LlmResponse>,
    cursor: AtomicUsize,
}

#[async_trait]
impl LlmProvider for ScriptProvider {
    async fn chat(&self, _req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        let i = self.cursor.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(i)
            .cloned()
            .ok_or_else(|| LlmError::FakeScript(format!("hết kịch bản tại lượt {i}")))
    }

    async fn chat_stream(
        &self,
        _req: ChatRequest<'_>,
    ) -> Result<beanagent_llm::LlmStream, LlmError> {
        use beanagent_types::LlmDelta;
        use futures_util::stream;

        let i = self.cursor.fetch_add(1, Ordering::SeqCst);
        let response = self
            .responses
            .get(i)
            .cloned()
            .ok_or_else(|| LlmError::FakeScript(format!("hết kịch bản tại lượt {i}")))?;
        let deltas = response
            .text
            .filter(|text| !text.is_empty())
            .map(|text| vec![LlmDelta::Text { text }])
            .unwrap_or_default();
        let finish = vec![
            LlmDelta::Usage {
                usage: response.usage,
            },
            LlmDelta::Stop {
                reason: response.stop,
            },
        ];
        let calls = response
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| {
                LlmDelta::ToolCall(beanagent_types::LlmToolCallDelta {
                    index,
                    id: Some(call.id),
                    name: Some(call.name),
                    arguments_delta: Some(call.args.to_string()),
                })
            });
        let items: Vec<Result<LlmDelta, LlmError>> =
            calls.chain(deltas).chain(finish).map(Ok).collect();
        Ok(Box::pin(stream::iter(items)))
    }

    fn name(&self) -> &'static str {
        "script"
    }
}

impl std::fmt::Debug for ScriptProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScriptProvider")
    }
}

/// Tool giả lập: đếm số lần gọi, cấu hình được mức rủi ro, ép lỗi và mô phỏng công việc dài.
struct ProbeTool {
    risk: Risk,
    fail: bool,
    /// Nếu đặt (> 0): tool "làm việc" rất lâu và run bị huỷ sau N ms — dùng để test huỷ.
    cancel_after_ms: AtomicU64,
    calls: AtomicUsize,
}

impl ProbeTool {
    /// Số lần tool đã được gọi.
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Lên lịch huỷ run sau `ms` milliseconds kể từ khi tool bắt đầu chạy.
    fn set_cancel_after(&self, ms: u64) {
        self.cancel_after_ms.store(ms, Ordering::Relaxed);
    }
}

#[async_trait]
impl Tool for ProbeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "probe",
            "tool dùng cho test",
            serde_json::json!({"type": "object"}),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        self.risk
    }

    async fn call(&self, ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let scheduled = self.cancel_after_ms.load(Ordering::Relaxed);
        if scheduled > 0 {
            // Mô phỏng công việc dài: hẹn huỷ run rồi "làm việc" 30 giây (sẽ bị cắt bởi
            // vòng lặp khi token bị huỷ — không bao giờ chờ hết 30 giây này trong test).
            let token = ctx.cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(scheduled)).await;
                token.cancel();
            });
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        if self.fail {
            Err(ToolError::Io("lỗi có chủ đích cho test".into()))
        } else {
            Ok("ok-probe".into())
        }
    }
}

impl std::fmt::Debug for ProbeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProbeTool")
    }
}

/// `RunIo` ghi event vào buffer; `confirm` trả quyết định đặt sẵn (mặc định: không phản hồi).
/// Kèm bộ đếm cho test policy M4: số lần `confirm` được gọi + `allow_in_session` từng lần.
struct TestIo {
    cancel: CancellationToken,
    decision: Option<Decision>,
    events: Arc<Mutex<Vec<String>>>,
    confirm_count: Arc<AtomicUsize>,
    allow_in_session_flags: Arc<Mutex<Vec<bool>>>,
}

impl Clone for TestIo {
    fn clone(&self) -> Self {
        Self {
            cancel: self.cancel.clone(),
            decision: self.decision,
            events: self.events.clone(),
            confirm_count: self.confirm_count.clone(),
            allow_in_session_flags: self.allow_in_session_flags.clone(),
        }
    }
}

impl TestIo {
    fn new(decision: Option<Decision>) -> Self {
        Self {
            cancel: CancellationToken::new(),
            decision,
            events: Arc::new(Mutex::new(Vec::new())),
            confirm_count: Arc::new(AtomicUsize::new(0)),
            allow_in_session_flags: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Io cho phép mọi confirm (quyết định ALLOW cho mọi mức rủi ro).
    fn allow_all() -> Self {
        Self::new(Some(Decision::Allow))
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl RunIo for TestIo {
    fn on_text(&self, text: &str) {
        self.events.lock().unwrap().push(format!("text:{text}"));
    }

    fn on_tool_start(&self, _id: &str, tool: &str, _risk: Risk, summary: &str, args: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("start:{tool}:{summary}:{args}"));
    }

    fn on_tool_end(&self, _id: &str, tool: &str, ok: bool, output: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("end:{tool}:{ok}:{output}"));
    }

    async fn confirm(
        &self,
        _id: &str,
        _tool: &str,
        _risk: Risk,
        _prompt: &str,
        allow_in_session: bool,
        _timeout: Duration,
    ) -> Option<Decision> {
        self.confirm_count.fetch_add(1, Ordering::SeqCst);
        self.allow_in_session_flags
            .lock()
            .unwrap()
            .push(allow_in_session);
        self.decision
    }

    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

fn cfg(max_steps: u32) -> Config {
    let mut c = Config::default();
    c.agent.max_steps = max_steps;
    c.security.tool_timeout_seconds = 5;
    c
}

fn text_resp(t: &str) -> LlmResponse {
    LlmResponse::text_only(t)
}

fn tool_resp(calls: Vec<ToolCall>) -> LlmResponse {
    LlmResponse::with_tool_calls(calls)
}

fn probe_call(id: &str) -> ToolCall {
    ToolCall::new(id, "probe", serde_json::json!({}))
}

/// Registry + workspace tạm. Trả về `TempDir` để test giữ thư mục sống đủ lâu
/// (khi `TempDir` bị drop, thư mục tạm bị xoá).
fn registry_with(tool: Arc<dyn Tool>) -> (tempfile::TempDir, ToolRegistry) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap());
    let mut reg = ToolRegistry::with_workspace(ws);
    reg.register(tool).unwrap();
    (dir, reg)
}

fn probe(risk: Risk, fail: bool) -> (Arc<ProbeTool>, tempfile::TempDir, ToolRegistry) {
    let t = Arc::new(ProbeTool {
        risk,
        fail,
        cancel_after_ms: AtomicU64::new(0),
        calls: AtomicUsize::new(0),
    });
    let (ws, reg) = registry_with(t.clone());
    (t, ws, reg)
}

async fn turn(
    responses: Vec<LlmResponse>,
    registry: &ToolRegistry,
    io: &TestIo,
    store: &MemoryStore,
    cfg: &Config,
) -> Result<String, AgentError> {
    let provider = ScriptProvider {
        responses,
        cursor: AtomicUsize::new(0),
    };
    let owned_io = Arc::new(io.clone());
    let perms = RolePermissions::unrestricted("test");

    run_turn(RunTurnArgs {
        store,
        registry,
        llm: &provider,
        config: cfg,
        session: SessionId::new(1),
        user_text: "bắt đầu".into(),
        io: owned_io.clone(),
        cancel: owned_io.cancel.clone(),
        // (M4) Test mặc định: không persist allow-in-session giữa các call trong test
        // khác — test policy riêng truyền Some(...) qua `turn_sec`.
        session_policy: None,
        audit: None,
        channel: "cli",
        skills_index: "",
        permissions: &perms,
        project: "default",
        alerts: None,
    })
    .await
}

async fn tool_messages(store: &MemoryStore) -> Vec<beanagent_types::Message> {
    store
        .history(SessionId::new(1), None, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|m| m.role == beanagent_types::Role::Tool)
        .collect()
}

/// 1. Kết thúc đúng: model trả text, không gọi tool.
#[tokio::test]
async fn finishes_when_model_answers_without_tools() {
    let (_, _ws, reg) = probe(Risk::Safe, false);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let out = turn(vec![text_resp("Xong rồi nhé")], &reg, &io, &store, &cfg(5))
        .await
        .unwrap();
    assert_eq!(out, "Xong rồi nhé");
    let hist = store.history(SessionId::new(1), None, 0).await.unwrap();
    assert_eq!(hist.len(), 2); // user + assistant
    assert!(tool_messages(&store).await.is_empty());
}

/// 2. Chạy tool rồi trả lời cuối; tool result không lỗi được ghi đủ.
#[tokio::test]
async fn runs_tool_then_final_answer() {
    let (t, _ws, reg) = probe(Risk::Safe, false);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let out = turn(
        vec![tool_resp(vec![probe_call("c1")]), text_resp("xong")],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    assert_eq!(t.count(), 1);
    assert_eq!(out, "xong");
    let msgs = tool_messages(&store).await;
    assert_eq!(msgs.len(), 1);
    assert!(!msgs[0].is_error);
    assert_eq!(msgs[0].tool_call_id.as_deref(), Some("c1"));
    // Event tool_start/tool_end đã phát cho UI.
    let events = io.events();
    assert!(events.iter().any(|e| e.starts_with("start:probe:")));
    assert!(events.iter().any(|e| e.starts_with("end:probe:true:")));
}

#[tokio::test]
async fn streaming_deltas_are_emitted_and_assembled_into_tool_call() {
    struct StreamProvider {
        calls: AtomicUsize,
    }
    impl std::fmt::Debug for StreamProvider {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("StreamProvider")
                .field("calls", &self.calls)
                .finish()
        }
    }
    #[async_trait]
    impl LlmProvider for StreamProvider {
        async fn chat(&self, _request: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse::text_only("không dùng"))
        }
        async fn chat_stream_with_model(
            &self,
            _request: ChatRequest<'_>,
            _model: &str,
        ) -> Result<beanagent_llm::LlmStream, LlmError> {
            use futures_util::stream;
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                let first = vec![
                    Ok(LlmDelta::Text {
                        text: "đang kiểm tra".into(),
                    }),
                    Ok(LlmDelta::ToolCall(LlmToolCallDelta {
                        index: 0,
                        id: Some("c-stream".into()),
                        name: Some("probe".into()),
                        arguments_delta: Some("{\"x\":".into()),
                    })),
                    Ok(LlmDelta::ToolCall(LlmToolCallDelta {
                        index: 0,
                        id: None,
                        name: None,
                        arguments_delta: Some("1}".into()),
                    })),
                    Ok(LlmDelta::Stop {
                        reason: StopReason::ToolUse,
                    }),
                ];
                return Ok(Box::pin(stream::iter(first)));
            }
            let second = vec![
                Ok(LlmDelta::Text {
                    text: "xong".into(),
                }),
                Ok(LlmDelta::Stop {
                    reason: StopReason::EndTurn,
                }),
            ];
            Ok(Box::pin(stream::iter(second)))
        }
        fn name(&self) -> &'static str {
            "stream"
        }
    }

    let (probe, _ws, reg) = probe(Risk::Safe, false);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let provider = StreamProvider {
        calls: AtomicUsize::new(0),
    };
    let perms = RolePermissions::unrestricted("test");

    let out = run_turn(RunTurnArgs {
        store: &store,
        registry: &reg,
        llm: &provider,
        config: &cfg(5),
        session: SessionId::new(1),
        user_text: "stream".into(),
        io: Arc::new(io.clone()),
        cancel: io.cancel.clone(),
        session_policy: None,
        audit: None,
        channel: "cli",
        skills_index: "",
        permissions: &perms,
        project: "default",
        alerts: None,
    })
    .await
    .unwrap();
    assert_eq!(out, "xong");
    assert_eq!(probe.count(), 1);
    let history = store.history(SessionId::new(1), None, 0).await.unwrap();
    let assistant_with_tool = history
        .iter()
        .find(|message| !message.tool_calls.is_empty())
        .unwrap();
    assert_eq!(assistant_with_tool.text.as_deref(), Some("đang kiểm tra"));
    assert_eq!(
        assistant_with_tool.tool_calls[0].args,
        serde_json::json!({"x": 1})
    );
}

/// 3. Dừng ở `max_steps`: mỗi bước đều phát tool call thì dừng đúng số bước.
#[tokio::test]
async fn stops_at_max_steps() {
    let (t, _ws, reg) = probe(Risk::Safe, false);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let responses: Vec<LlmResponse> = (0..3)
        .map(|i| tool_resp(vec![probe_call(&format!("c{i}"))]))
        .collect();
    let out = turn(responses, &reg, &io, &store, &cfg(3)).await.unwrap();
    assert_eq!(t.count(), 3);
    assert!(out.contains("giới hạn số bước"), "{out}");
    let history = store.history(SessionId::new(1), None, 0).await.unwrap();
    assert_eq!(
        history.last().and_then(|m| m.text.as_deref()),
        Some(out.as_str())
    );
}

#[tokio::test]
async fn stops_with_persisted_notice_when_daily_budget_is_exceeded() {
    let (_ws, reg) = registry_with(Arc::new(ProbeTool {
        risk: Risk::Safe,
        fail: false,
        cancel_after_ms: AtomicU64::new(0),
        calls: AtomicUsize::new(0),
    }));
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let mut config = cfg(5);
    config.security.daily_token_budget = 10;
    let mut response = text_resp("provider đã trả lời");
    response.usage = Usage {
        input_tokens: 6,
        output_tokens: 5,
    };
    let out = turn(vec![response], &reg, &io, &store, &config)
        .await
        .unwrap();
    assert!(out.contains("ngân sách token/ngày"), "{out}");
    let history = store.history(SessionId::new(1), None, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history.last().and_then(|m| m.text.as_deref()),
        Some(out.as_str())
    );
}

fn files_registry() -> (tempfile::TempDir, ToolRegistry) {
    let dir = tempfile::tempdir().unwrap();
    let ws = Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap());
    let mut reg = ToolRegistry::with_workspace(ws);
    for t in beanagent_tools::builtin::file_tools() {
        reg.register(t).unwrap();
    }
    (dir, reg)
}

/// 4. Tool lỗi (file không tồn tại) không làm crash — thành tool result `is_error`.
#[tokio::test]
async fn tool_error_becomes_is_error_result() {
    let (_ws, reg) = files_registry();
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let call = ToolCall::new("c1", "read_file", serde_json::json!({"path": "thiếu.txt"}));
    let out = turn(
        vec![tool_resp(vec![call]), text_resp("đã xử lý")],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    assert_eq!(out, "đã xử lý");
    let msgs = tool_messages(&store).await;
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].is_error);
    assert!(
        msgs[0]
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("thiếu.txt")
    );
}

/// 5. Tool không tồn tại → lỗi rõ ràng, không crash.
#[tokio::test]
async fn unknown_tool_returns_clear_error() {
    let (_, _ws, reg) = probe(Risk::Safe, false);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let call = ToolCall::new("c1", "tool_lạ", serde_json::json!({}));
    turn(
        vec![tool_resp(vec![call]), text_resp("ok")],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    let msgs = tool_messages(&store).await;
    assert!(msgs[0].is_error);
    assert!(
        msgs[0]
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("tool_lạ")
    );
}

/// 6. Tham số sai kiểu và tham số thừa đều bị từ chối (`deny_unknown_fields`).
#[tokio::test]
async fn wrong_and_unknown_args_rejected() {
    let (_ws, reg) = files_registry();
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let calls = vec![
        ToolCall::new("c1", "read_file", serde_json::json!({"path": 123})), // sai kiểu
        ToolCall::new(
            "c2",
            "read_file",
            serde_json::json!({"path": "a", "evil": true}),
        ), // thừa
    ];
    turn(
        vec![tool_resp(calls), text_resp("ok")],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    let msgs = tool_messages(&store).await;
    assert_eq!(msgs.len(), 2);
    assert!(msgs.iter().all(|m| m.is_error));
    assert!(msgs.iter().any(|m| m.tool_call_id.as_deref() == Some("c1")));
    assert!(msgs.iter().any(|m| m.tool_call_id.as_deref() == Some("c2")));
}

/// 7. Cắt output > 20.000 ký tự: không panic với tiếng Việt/emoji, cắt đúng ranh giới.
#[tokio::test]
async fn truncate_output_utf8_safe_viet_emoji() {
    // Tool trả chuỗi dài gồm emoji 4 byte xen tiếng Việt.
    struct Big {
        body: String,
    }
    #[async_trait]
    impl Tool for Big {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "big",
                "trả chuỗi dài",
                serde_json::json!({"type": "object"}),
            )
        }
        fn risk(&self, _a: &serde_json::Value) -> Risk {
            Risk::Safe
        }
        async fn call(
            &self,
            _ctx: &ToolCtx,
            _args: serde_json::Value,
        ) -> Result<String, ToolError> {
            Ok(self.body.clone())
        }
    }
    impl std::fmt::Debug for Big {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Big")
        }
    }
    let (_ws, reg) = registry_with(Arc::new(Big {
        body: "hả🦀ệ".repeat(6000),
    })); // 24_000 ký tự
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    turn(
        vec![
            tool_resp(vec![ToolCall::new("c1", "big", serde_json::json!({}))]),
            text_resp("ok"),
        ],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    let msgs = tool_messages(&store).await;
    let text = msgs[0].text.clone().unwrap_or_default();
    assert!(text.contains("đã cắt"), "{text}");
    // 20_000 ký tự thân + phần ghi chú.
    assert!(text.chars().count() <= 20_100);
    assert!(text.chars().count() >= 20_000);
    // Không có replacement char (điểm cắt lệch byte).
    assert!(!text.contains('\u{FFFD}'));
}

/// 8. Chống lặp: 2 lần thất bại giống nhau → gợi ý trong result; lần 3 chặn và dừng run.
#[tokio::test]
async fn repeat_failure_hint_then_stop_on_third() {
    let (t, _ws, reg) = probe(Risk::Safe, true); // tool luôn lỗi
    let io = TestIo::allow_all();
    let store = MemoryStore::new();
    let responses: Vec<LlmResponse> = (0..3)
        .map(|i| tool_resp(vec![probe_call(&format!("c{i}"))]))
        .collect();
    let out = turn(responses, &reg, &io, &store, &cfg(10)).await;
    // Run kết thúc bằng lỗi chống lặp.
    assert!(matches!(out, Err(AgentError::RepeatFailure(_))), "{out:?}");
    // Chỉ gọi 2 lần: lần 3 bị chặn trước khi thực thi.
    assert_eq!(t.count(), 2);
    // Tool result cuối mang gợi ý "thử cách khác".
    let msgs = tool_messages(&store).await;
    let last = msgs.last().unwrap();
    assert!(last.is_error);
    assert!(
        last.text
            .as_deref()
            .unwrap_or_default()
            .contains("thử cách khác")
    );
}

/// 9. Confirm — cho phép: tool chạy, result không lỗi.
#[tokio::test]
async fn confirm_allowed_runs_tool() {
    let (t, _ws, reg) = probe(Risk::Confirm, false);
    let io = TestIo::allow_all();
    let store = MemoryStore::new();
    turn(
        vec![tool_resp(vec![probe_call("c1")]), text_resp("ok")],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    assert_eq!(t.count(), 1);
    let msgs = tool_messages(&store).await;
    assert!(!msgs[0].is_error);
    assert_eq!(msgs[0].text.as_deref(), Some("ok-probe"));
}

/// 10. Confirm — từ chối: tool không chạy, result lỗi "từ chối".
#[tokio::test]
async fn confirm_denied_yields_error_result() {
    let (t, _ws, reg) = probe(Risk::Confirm, false);
    let io = TestIo::new(Some(Decision::Deny));
    let store = MemoryStore::new();
    turn(
        vec![tool_resp(vec![probe_call("c1")]), text_resp("ok")],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await
    .unwrap();
    assert_eq!(t.count(), 0);
    let msgs = tool_messages(&store).await;
    assert!(msgs[0].is_error);
    assert!(
        msgs[0]
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("từ chối")
    );
}

/// 11. Huỷ giữa chừng: vẫn ghi tool result "[bị người dùng huỷ]" để lịch sử giữ cặp hợp lệ.
#[tokio::test]
async fn cancellation_persists_placeholder_results() {
    let (t, _ws, reg) = probe(Risk::Safe, false);
    // Huỷ 50 ms sau khi tool bắt đầu; tool "làm việc" 30 s nên chắc chắn bị cắt giữa chừng.
    t.set_cancel_after(50);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let res = turn(
        vec![tool_resp(vec![probe_call("c1")])],
        &reg,
        &io,
        &store,
        &cfg(5),
    )
    .await;
    assert!(res.is_err(), "run phải kết thúc khi bị huỷ: {res:?}");
    let msgs = tool_messages(&store).await;
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].is_error);
    assert!(
        msgs[0]
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("bị người dùng huỷ")
    );
}

// ---------------------------------------------------------------------------
// M5/D8.10 — system prompt gửi đúng một lần
// ---------------------------------------------------------------------------

/// Một lượt gọi LLM đã ghi lại để kiểm tra system prompt.
#[derive(Debug)]
struct RecordedRequest {
    system: String,
    roles: Vec<Role>,
    first_text: Option<String>,
    /// System prompt có bị lặp thành message nào không.
    system_leaked_into_messages: bool,
}

/// Provider ghi lại request rồi trả về text tĩnh (nên mỗi lượt chỉ chạy một vòng).
#[derive(Debug)]
struct RecordingProvider {
    seen: Mutex<Vec<RecordedRequest>>,
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.seen.lock().unwrap().push(RecordedRequest {
            system: req.system.to_string(),
            roles: req.messages.iter().map(|message| message.role).collect(),
            first_text: req
                .messages
                .first()
                .and_then(|message| message.text.clone()),
            system_leaked_into_messages: req.messages.iter().any(|message| {
                message
                    .text
                    .as_deref()
                    .is_some_and(|text| text.starts_with("You are "))
            }),
        });
        Ok(LlmResponse::text_only("xong"))
    }

    fn name(&self) -> &'static str {
        "recording"
    }
}

/// System prompt phải nằm ở `ChatRequest.system` và **không** bị nhân bản thành message
/// `User` (M3 từng làm vậy — tốn token gấp đôi). Message đầu tiên phải là tin người dùng
/// thật, đúng mục 8.2.
#[tokio::test]
async fn system_prompt_is_sent_once_via_system_field() {
    let (_probe, _ws, reg) = probe(Risk::Safe, false);
    let io = TestIo::new(None);
    let store = MemoryStore::new();
    let provider = RecordingProvider {
        seen: Mutex::new(Vec::new()),
    };

    let owned_io = Arc::new(io);
    let perms = RolePermissions::unrestricted("test");

    let out = run_turn(RunTurnArgs {
        store: &store,
        registry: &reg,
        llm: &provider,
        config: &cfg(5),
        session: SessionId::new(1),
        user_text: "xin chào".into(),
        io: owned_io.clone(),
        cancel: owned_io.cancel.clone(),
        session_policy: None,
        audit: None,
        channel: "cli",
        skills_index: "",
        permissions: &perms,
        project: "default",
        alerts: None,
    })
    .await
    .unwrap();
    assert_eq!(out, "xong");

    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "trả lời ngay ⇒ chỉ một lượt gọi LLM");
    let first = seen.first().unwrap();
    assert!(
        first.system.contains("You are"),
        "system prompt phải nằm ở ChatRequest.system"
    );
    assert!(
        !first.system_leaked_into_messages,
        "system prompt bị nhân bản thành message"
    );
    assert_eq!(first.roles.first().copied(), Some(Role::User));
    assert_eq!(first.first_text.as_deref(), Some("xin chào"));
}
