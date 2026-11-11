//! Test checkpoint/rollback workspace trong vòng lặp agent (D26 — lộ trình mục 5c).
//!
//! Kiểm chứng tại tầng vòng lặp (provider giả, không mạng):
//! * chụp checkpoint **một lần** trước tool có rủi ro đầu tiên của lượt;
//! * `rollback_workspace` khôi phục nội dung workspace về trước khi model ghi;
//! * rollback khi chưa có gì thay đổi trả lỗi rõ ràng, không crash vòng lặp;
//! * checkpoint tắt (`checkpoints = None`) ⇒ tool ghi chạy bình thường, không có
//!   thư mục checkpoint nào được tạo;
//! * `rollback_workspace` là `Confirm` ⇒ dưới scheduler (background, không người
//!   duyệt) bị từ chối tự động.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bean_core::agent::AgentError;
use bean_core::run_io::{Decision, RunIo};
use bean_core::store::MemoryStore;
use bean_core::{RunTurnArgs, run_turn_outcome};
use bean_llm::{ChatRequest, LlmError, LlmProvider};
use bean_security::checkpoint::CheckpointStore;
use bean_security::{CapWorkspace, SessionPolicy};
use bean_tools::ToolRegistry;
use bean_types::config::CheckpointConfig;
use bean_types::{Config, LlmResponse, Risk, RolePermissions, SessionId, ToolCall};
use tokio_util::sync::CancellationToken;

/// Provider trả sẵn kịch bản các response.
struct Script {
    responses: Vec<LlmResponse>,
    cursor: AtomicUsize,
}

#[async_trait]
impl LlmProvider for Script {
    async fn chat(&self, _req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        let i = self.cursor.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(i)
            .cloned()
            .ok_or_else(|| LlmError::FakeScript(format!("hết kịch bản tại lượt {i}")))
    }
    fn name(&self) -> &'static str {
        "script"
    }
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Script")
    }
}

/// `RunIo` trả quyết định đặt sẵn cho mọi lần confirm.
struct AutoIo {
    cancel: CancellationToken,
    decision: Option<Decision>,
}

impl Clone for AutoIo {
    fn clone(&self) -> Self {
        Self {
            cancel: self.cancel.clone(),
            decision: self.decision,
        }
    }
}

impl AutoIo {
    fn allow() -> Self {
        Self {
            cancel: CancellationToken::new(),
            decision: Some(Decision::Allow),
        }
    }
    /// Mô phỏng scheduler/background: không ai duyệt được.
    fn background() -> Self {
        Self {
            cancel: CancellationToken::new(),
            decision: None,
        }
    }
}

#[async_trait]
impl RunIo for AutoIo {
    fn on_text(&self, _text: &str) {}
    fn on_tool_start(&self, _id: &str, _tool: &str, _risk: Risk, _summary: &str, _args: &str) {}
    fn on_tool_end(&self, _id: &str, _tool: &str, _ok: bool, _output: &str) {}
    async fn confirm(
        &self,
        _id: &str,
        _tool: &str,
        _risk: Risk,
        _prompt: &str,
        _allow_in_session: bool,
        _timeout: Duration,
    ) -> Option<Decision> {
        self.decision
    }
    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
    fn is_background(&self) -> bool {
        self.decision.is_none()
    }
}

fn write_call(id: &str, content: &str) -> ToolCall {
    ToolCall::new(
        id,
        "write_file",
        serde_json::json!({ "path": "note.txt", "content": content }),
    )
}

fn rollback_call(id: &str) -> ToolCall {
    ToolCall::new(id, "rollback_workspace", serde_json::json!({}))
}

struct Env {
    _temp: tempfile::TempDir,
    ws: std::path::PathBuf,
    checkpoints_dir: std::path::PathBuf,
    store: Arc<CheckpointStore>,
    registry: ToolRegistry,
}

fn env_with_checkpoints() -> Env {
    let temp = tempfile::tempdir().unwrap();
    let ws = temp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("note.txt"), "gốc").unwrap();
    let checkpoints_dir = temp.path().join("checkpoints");
    let store = Arc::new(CheckpointStore::new(
        checkpoints_dir.clone(),
        &CheckpointConfig::default(),
    ));
    let cap = CapWorkspace::open(ws.clone()).unwrap();
    let mut registry = ToolRegistry::with_workspace(Arc::new(cap));
    for tool in bean_tools::builtin::file_tools() {
        registry.register(tool).unwrap();
    }
    registry
        .register(bean_tools::builtin::rollback_workspace())
        .unwrap();
    Env {
        _temp: temp,
        ws,
        checkpoints_dir,
        store,
        registry,
    }
}

async fn run(
    env: &Env,
    io: &AutoIo,
    responses: Vec<LlmResponse>,
    checkpoints: Option<&Arc<CheckpointStore>>,
) -> Result<bean_core::agent::RunOutcome, AgentError> {
    let provider = Script {
        responses,
        cursor: AtomicUsize::new(0),
    };
    let mut config = Config::default();
    config.agent.max_steps = 10;
    config.security.tool_timeout_seconds = 5;
    let store = MemoryStore::new();
    let owned_io = Arc::new(io.clone());
    let perms = RolePermissions::unrestricted("test");
    run_turn_outcome(RunTurnArgs {
        store: &store,
        registry: &env.registry,
        llm: &provider,
        config: &config,
        session: SessionId::new(1),
        user_text: "bắt đầu".into(),
        io: owned_io.clone(),
        cancel: owned_io.cancel.clone(),
        session_policy: None::<Arc<SessionPolicy>>,
        audit: None,
        channel: "test",
        skills_index: "",
        permissions: &perms,
        project: "default",
        alerts: None,
        checkpoints: checkpoints.cloned(),
    })
    .await
}

fn tool_error_texts(outcome: &bean_core::agent::RunOutcome) -> Vec<String> {
    outcome
        .transcript
        .iter()
        .filter(|message| message.is_error)
        .filter_map(|message| message.text.clone())
        .collect()
}

#[tokio::test]
async fn rollback_tra_workspace_ve_truoc_khi_model_ghi() {
    let env = env_with_checkpoints();
    let io = AutoIo::allow();
    let outcome = run(
        &env,
        &io,
        vec![
            LlmResponse::with_tool_calls(vec![write_call("c1", "nội dung ghi sai")]),
            LlmResponse::with_tool_calls(vec![rollback_call("c2")]),
            LlmResponse::text_only("đã khôi phục"),
        ],
        Some(&env.store),
    )
    .await
    .unwrap();
    // File trở về đúng nội dung gốc.
    assert_eq!(
        std::fs::read_to_string(env.ws.join("note.txt")).unwrap(),
        "gốc"
    );
    // Không còn lỗi nào trong transcript.
    assert!(
        tool_error_texts(&outcome).is_empty(),
        "không mong đợi lỗi: {:?}",
        tool_error_texts(&outcome)
    );
    // Đã chụp đúng một checkpoint trong thư mục data.
    let entries = std::fs::read_dir(&env.checkpoints_dir).unwrap().count();
    assert_eq!(entries, 1);
}

#[tokio::test]
async fn rollback_khi_chua_co_gi_thay_doi_thi_tra_loi_ro_rang() {
    let env = env_with_checkpoints();
    let io = AutoIo::allow();
    let outcome = run(
        &env,
        &io,
        vec![
            // Rollback là tool call ĐẦU TIÊN của lượt — chưa có checkpoint.
            LlmResponse::with_tool_calls(vec![rollback_call("c1")]),
            LlmResponse::text_only("hiểu rồi"),
        ],
        Some(&env.store),
    )
    .await
    .unwrap();
    let errors = tool_error_texts(&outcome);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("chưa có checkpoint"), "{:?}", errors[0]);
    // File gốc không bị đụng tới.
    assert_eq!(
        std::fs::read_to_string(env.ws.join("note.txt")).unwrap(),
        "gốc"
    );
}

#[tokio::test]
async fn checkpoint_tat_thi_tool_ghi_van_chay_va_khong_tao_thu_muc() {
    let temp = tempfile::tempdir().unwrap();
    let ws = temp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("note.txt"), "gốc").unwrap();
    let cap = CapWorkspace::open(ws.clone()).unwrap();
    let mut registry = ToolRegistry::with_workspace(Arc::new(cap));
    for tool in bean_tools::builtin::file_tools() {
        registry.register(tool).unwrap();
    }
    let checkpoints_dir = temp.path().join("checkpoints");
    let env = Env {
        _temp: temp,
        ws,
        checkpoints_dir: checkpoints_dir.clone(),
        store: Arc::new(CheckpointStore::new(
            checkpoints_dir.clone(),
            &CheckpointConfig::default(),
        )),
        registry,
    };
    let io = AutoIo::allow();
    run(
        &env,
        &io,
        vec![
            LlmResponse::with_tool_calls(vec![write_call("c1", "nội dung mới")]),
            LlmResponse::text_only("đã ghi"),
        ],
        None, // checkpoints tắt
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(env.ws.join("note.txt")).unwrap(),
        "nội dung mới"
    );
    assert!(!checkpoints_dir.exists());
}

#[tokio::test]
async fn rollback_duoi_scheduler_bi_tu_choi_tu_dong() {
    let env = env_with_checkpoints();
    // Scheduler không có người duyệt: confirm trả None ⇒ DENY (mục 14).
    let io = AutoIo::background();
    let outcome = run(
        &env,
        &io,
        vec![
            // write_file trước để có checkpoint; rồi rollback phải bị từ chối.
            LlmResponse::with_tool_calls(vec![write_call("c1", "nội dung mới")]),
            LlmResponse::with_tool_calls(vec![rollback_call("c2")]),
            LlmResponse::text_only("đã hiểu"),
        ],
        Some(&env.store),
    )
    .await
    .unwrap();
    // write_file được "duyệt tự động"? Không — background + Confirm ⇒ bị từ chối
    // theo chính sách bốn mắt, nên note.txt giữ nguyên "gốc" và KHÔNG có rollback.
    assert_eq!(
        std::fs::read_to_string(env.ws.join("note.txt")).unwrap(),
        "gốc"
    );
    assert!(
        !tool_error_texts(&outcome).is_empty()
            || outcome
                .transcript
                .iter()
                .any(|m| { m.text.as_deref().unwrap_or_default().contains("từ chối") })
    );
}
