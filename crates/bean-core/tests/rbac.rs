//! M21 — Project profile + RBAC + Developer/QA role (Plan.md mục 5).
//!
//! Bằng chứng cho **4 test bắt buộc** của milestone:
//!
//! 1. `finance_readonly_sees_no_tool_outside_billing_read` — role `finance-readonly` không
//!    thấy tool nào ngoài tag `billing-read` trong **payload gửi LLM** (M21.5).
//! 2. `user_without_role_is_no_access_and_sees_no_tool` — user không có trong
//!    `agent.user_roles` là `no-access`, **không gọi được tool nào kể cả tool an toàn cũ**
//!    (D10.2, fail-closed).
//! 3. `qa_cannot_see_or_call_dev_write_tool` — nguyên tắc four-eyes (M21.6).
//! 4. `two_projects_do_not_mix_memory_md` — hai project không lẫn `MEMORY.md` (M21.1).
//!
//! Ngoài ra có test cho chốt chặn thứ hai ở tầng thực thi (model bịa tool ngoài quyền).
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bean_core::run_io::{Decision, RunIo};
use bean_core::{RunTurnArgs, run_turn};
use bean_llm::{ChatRequest, LlmError, LlmProvider, LlmStream};
use bean_memory::{MemoryStore, Store};
use bean_security::CapWorkspace;
use bean_tools::{Tool, ToolAccess, ToolCtx, ToolError, ToolRegistry};
use bean_types::config::{ProjectConfig, RoleConfig};
use bean_types::{
    Config, LlmDelta, LlmResponse, Risk, RolePermissions, SessionId, ToolCall, ToolSpec,
};
use futures_util::stream;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Provider trả về các tool call theo kịch bản và **ghi lại** danh sách tool từng lần —
/// đó là bằng chứng trực tiếp cho "lọc TRƯỚC khi dựng request tới LLM" (M21.5).
#[derive(Debug)]
struct SpyProvider {
    responses: Vec<LlmResponse>,
    cursor: AtomicUsize,
    seen_tools: std::sync::Mutex<Vec<Vec<String>>>,
}

impl SpyProvider {
    fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses,
            cursor: AtomicUsize::new(0),
            seen_tools: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn record(&self, req: &ChatRequest<'_>) {
        self.seen_tools
            .lock()
            .unwrap()
            .push(req.tools.iter().map(|t| t.name.clone()).collect());
    }
}

#[async_trait::async_trait]
impl LlmProvider for SpyProvider {
    fn name(&self) -> &'static str {
        "spy"
    }

    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.record(&req);
        Ok(self.next())
    }

    async fn chat_stream(&self, req: ChatRequest<'_>) -> Result<LlmStream, LlmError> {
        self.record(&req);
        let deltas = self
            .next()
            .text
            .filter(|t| !t.is_empty())
            .map(|t| vec![LlmDelta::Text { text: t }])
            .unwrap_or_default();
        Ok(Box::pin(stream::iter(deltas.into_iter().map(Ok))))
    }
}

impl SpyProvider {
    fn next(&self) -> LlmResponse {
        self.responses
            .get(self.cursor.fetch_add(1, Ordering::SeqCst))
            .cloned()
            .unwrap_or_else(|| LlmResponse::text_only("xong"))
    }
}

/// Tool giả để kiểm tra lọc theo tag, không cần file/docker thật.
struct TagTool {
    name: &'static str,
    tags: Vec<&'static str>,
}

#[async_trait::async_trait]
impl Tool for TagTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(self.name, "tool giả", serde_json::json!({"type": "object"}))
    }
    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }
    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            // Tag đến từ cấu hình chạy được, không phải literal.
            required_tags: Cow::Owned(self.tags.clone()),
            ..ToolAccess::default()
        }
    }
    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        Ok("ok".into())
    }
}

fn tag_tool(name: &'static str, tags: &[&'static str]) -> Arc<dyn Tool> {
    Arc::new(TagTool {
        name,
        tags: tags.to_vec(),
    })
}

/// Registry chứa đủ các tag domain của Plan.md mục 2b.
fn domain_registry() -> (TempDir, ToolRegistry) {
    let dir = TempDir::new().unwrap();
    let ws = Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap());
    let mut reg = ToolRegistry::with_workspace(ws);
    reg.register(tag_tool("chat_tool", &[])).unwrap();
    reg.register(tag_tool("billing_read", &["billing-read"]))
        .unwrap();
    reg.register(tag_tool("infra_read", &["infra-read"]))
        .unwrap();
    reg.register(tag_tool("dev_write", &["dev-write"])).unwrap();
    reg.register(tag_tool("run_shell", &["dev-write", "infra-scan"]))
        .unwrap();
    (dir, reg)
}

fn role(name: &str, tags: &[&str], forbid: &[&str]) -> RoleConfig {
    RoleConfig {
        name: name.into(),
        tool_tags: tags.iter().map(|s| (*s).to_string()).collect(),
        allowed_tool_tags: vec![],
        forbid_tags: forbid.iter().map(|s| (*s).to_string()).collect(),
        context_budget_tokens: None,
        daily_token_budget: None,
    }
}

/// Bảng `[[roles]]` + `agent.user_roles` theo đúng Plan.md mục 2/2b.
fn with_roles(config: &mut Config) {
    config.roles = vec![
        role("admin", &["*"], &[]),
        role("it-security", &["infra-read", "infra-scan"], &[]),
        role("finance-readonly", &["billing-read"], &[]),
        role("developer", &["dev-write"], &[]),
        // (M21.6) four-eyes: role review không bao giờ được cấp quyền ghi code.
        role("qa", &["dev-read", "test-run"], &["dev-write"]),
    ];
    config.agent.user_roles = [
        ("web:admin".to_string(), "admin".to_string()),
        ("cli:local".to_string(), "developer".to_string()),
    ]
    .into_iter()
    .collect();
}

fn base_config(dir: &TempDir) -> Config {
    let mut config = Config::default();
    config.agent.workspace = dir.path().to_path_buf();
    config.agent.allowed_users = vec!["web:admin".into(), "cli:local".into()];
    config
}

/// `RunIo` im lặng, cho phép mọi tool Confirm (để test tập trung vào RBAC).
struct SilentIo {
    cancel: CancellationToken,
}

#[async_trait::async_trait]
impl RunIo for SilentIo {
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
        _timeout: std::time::Duration,
    ) -> Option<Decision> {
        Some(Decision::Allow)
    }
    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

/// Chạy một lượt với quyền cho trước, trả `(text, danh sách tool từng lần thấy)`.
async fn run_once(
    config: &Config,
    reg: &ToolRegistry,
    perms: &RolePermissions,
    responses: Vec<LlmResponse>,
    project: &str,
) -> (String, Vec<Vec<String>>) {
    let store = MemoryStore::new();
    let provider = SpyProvider::new(responses);
    let io = Arc::new(SilentIo {
        cancel: CancellationToken::new(),
    });
    let text = run_turn(RunTurnArgs {
        store: &store,
        registry: reg,
        llm: &provider,
        config,
        session: SessionId::new(1),
        user_text: "chào".into(),
        io: io.clone(),
        cancel: io.cancel.clone(),
        session_policy: None,
        audit: None,
        channel: "cli",
        skills_index: "",
        permissions: perms,
        project,
        alerts: None,
    })
    .await
    .unwrap();
    let seen = provider.seen_tools.lock().unwrap().clone();
    (text, seen)
}

// ---------------------------------------------------------------------------
// Test 1: finance-readonly không thấy tool ngoài tag billing-read (M21.5)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn finance_readonly_sees_no_tool_outside_billing_read() {
    let (dir, reg) = domain_registry();
    let mut config = base_config(&dir);
    with_roles(&mut config);

    // User chưa được map thì phải là no-access (kiểm trước khi cấp quyền).
    assert_eq!(config.permissions_for("telegram:ke_toan").role, "no-access");

    config
        .agent
        .user_roles
        .insert("telegram:ke_toan".into(), "finance-readonly".into());
    let perms = config.permissions_for("telegram:ke_toan");
    assert_eq!(perms.role, "finance-readonly");

    let (_text, seen) = run_once(
        &config,
        &reg,
        &perms,
        vec![LlmResponse::text_only("xong")],
        "default",
    )
    .await;

    let tools = &seen[0];
    assert!(
        tools.contains(&"billing_read".to_string()),
        "phải thấy tool billing-read: {tools:?}"
    );
    for forbidden in ["infra_read", "dev_write", "run_shell"] {
        assert!(
            !tools.contains(&forbidden.to_string()),
            "finance-readonly không được thấy `{forbidden}`: {tools:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: user không có trong user_roles ⇒ no-access, không thấy tool nào (D10.2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn user_without_role_is_no_access_and_sees_no_tool() {
    let (dir, reg) = domain_registry();
    let mut config = base_config(&dir);
    with_roles(&mut config);

    let perms = config.permissions_for("telegram:nguoi_la");
    assert_eq!(perms.role, "no-access");
    assert!(perms.deny_all, "no-access phải là deny-all");

    let (_text, seen) = run_once(
        &config,
        &reg,
        &perms,
        vec![LlmResponse::text_only("xong")],
        "default",
    )
    .await;

    assert!(
        seen[0].is_empty(),
        "no-access không được thấy tool nào, kể cả tool untagged/an toàn: {:?}",
        seen[0]
    );
}

// ---------------------------------------------------------------------------
// Test 3: nguyên tắc four-eyes — qa không thấy/không gọi được tool dev-write (M21.6)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn qa_cannot_see_or_call_dev_write_tool() {
    let (dir, reg) = domain_registry();
    let mut config = base_config(&dir);
    with_roles(&mut config);
    config
        .agent
        .user_roles
        .insert("cli:local".into(), "qa".into());

    let perms = config.permissions_for("cli:local");
    assert_eq!(perms.role, "qa");

    // (a) Không thấy tool `dev-write` trong payload gửi LLM.
    let (_text, seen) = run_once(
        &config,
        &reg,
        &perms,
        vec![LlmResponse::text_only("xong")],
        "default",
    )
    .await;
    assert!(
        !seen[0].contains(&"dev_write".to_string()),
        "qa không được thấy dev_write: {:?}",
        seen[0]
    );
    assert!(
        !seen[0].contains(&"run_shell".to_string()),
        "qa không được thấy run_shell (nó cũng sửa được code): {:?}",
        seen[0]
    );

    // (b) Chốt chặn thứ hai: model bịa tên tool ngoài quyền vẫn **không chạy được**.
    let store = MemoryStore::new();
    let provider = SpyProvider::new(vec![
        LlmResponse {
            text: None,
            tool_calls: vec![ToolCall::new("c1", "dev_write", serde_json::json!({}))],
            stop: bean_types::StopReason::ToolUse,
            usage: Default::default(),
        },
        LlmResponse::text_only("không chạy được"),
    ]);
    let io = Arc::new(SilentIo {
        cancel: CancellationToken::new(),
    });
    run_turn(RunTurnArgs {
        store: &store,
        registry: &reg,
        llm: &provider,
        config: &config,
        session: SessionId::new(2),
        user_text: "hãy sửa code".into(),
        io: io.clone(),
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

    let history = store.history(SessionId::new(2), None, 0).await.unwrap();
    let tool_msg = history
        .iter()
        .find(|m| m.role == bean_types::Role::Tool)
        .expect("phải có tool result để giữ cặp hợp lệ");
    assert!(tool_msg.is_error, "tool call ngoài quyền phải là lỗi");
    assert!(
        tool_msg
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("không có quyền"),
        "thông báo phải nói rõ bị chặn vì quyền, got {:?}",
        tool_msg.text
    );
}

// ---------------------------------------------------------------------------
// Test 4: hai project không lẫn MEMORY.md (M21.1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_projects_do_not_mix_memory_md() {
    let dir = TempDir::new().unwrap();
    let project_a = dir.path().join("project-a");
    let project_b = dir.path().join("project-b");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();
    std::fs::write(project_a.join("MEMORY.md"), "bộ nhớ dự án A").unwrap();
    std::fs::write(project_b.join("MEMORY.md"), "bộ nhớ dự án B").unwrap();

    let mut config = Config::default();
    config.agent.workspace = project_a.clone();
    config.projects = vec![ProjectConfig {
        name: "project-b".into(),
        workspace: project_b.clone(),
    }];
    config.validate().unwrap();

    let mut reg =
        ToolRegistry::with_workspace(Arc::new(CapWorkspace::open(project_a.clone()).unwrap()));
    reg.set_project_workspace(
        "project-b",
        Arc::new(CapWorkspace::open(project_b.clone()).unwrap()),
    );

    let perms = RolePermissions::unrestricted("test");
    let store = MemoryStore::new();

    for (index, (project, expected, other)) in [
        ("default", "bộ nhớ dự án A", "bộ nhớ dự án B"),
        ("project-b", "bộ nhớ dự án B", "bộ nhớ dự án A"),
    ]
    .into_iter()
    .enumerate()
    {
        let session = SessionId::new(i64::try_from(index + 1).unwrap());
        let io = Arc::new(SilentIo {
            cancel: CancellationToken::new(),
        });
        run_turn(RunTurnArgs {
            store: &store,
            registry: &reg,
            llm: &SpyProvider::new(vec![LlmResponse::text_only("xong")]),
            config: &config,
            session,
            user_text: "bạn nhớ gì?".into(),
            io: io.clone(),
            cancel: io.cancel.clone(),
            session_policy: None,
            audit: None,
            channel: "cli",
            skills_index: "",
            permissions: &perms,
            project,
            alerts: None,
        })
        .await
        .unwrap();

        // Dựng lại đúng system prompt mà agent loop đã gửi cho project này.
        let ws = reg.workspace_for(project);
        let ctx = bean_core::context::build_for_project(
            &store,
            &config,
            session,
            ws.as_deref(),
            "",
            config.agent.context_budget_tokens,
            &perms.role,
        )
        .await
        .unwrap();

        assert!(
            ctx.system.contains(expected),
            "project `{project}` phải nạp MEMORY.md của chính nó (`{expected}`): {}",
            ctx.system
        );
        assert!(
            !ctx.system.contains(other),
            "project `{project}` KHÔNG được thấy MEMORY.md của project khác (`{other}`)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test bổ sung: cấu hình RBAC sai phải bị validate chặn (M21.2/M21.6)
// ---------------------------------------------------------------------------

#[test]
fn config_rejects_role_that_is_granted_a_forbidden_tag() {
    let dir = TempDir::new().unwrap();
    let mut config = base_config(&dir);
    with_roles(&mut config);
    // Cố lỡ tay cấp `dev-write` cho role `qa` dù nó khai báo forbid.
    let qa = config.roles.iter_mut().find(|r| r.name == "qa").unwrap();
    qa.tool_tags.push("dev-write".into());

    let err = config.validate().unwrap_err().to_string();
    assert!(
        err.contains("four-eyes"),
        "phải báo lỗi four-eyes, got: {err}"
    );
}

#[test]
fn config_rejects_user_role_pointing_to_missing_role() {
    let dir = TempDir::new().unwrap();
    let mut config = base_config(&dir);
    config
        .agent
        .user_roles
        .insert("cli:local".into(), "khong-ton-tai".into());
    let err = config.validate().unwrap_err().to_string();
    assert!(err.contains("không tồn tại"), "got: {err}");
}

#[test]
fn rbac_disabled_keeps_legacy_behavior_for_every_allowed_user() {
    // D10.3: không khai báo `user_roles` ⇒ hành vi cũ, không đột nhiên mất tool.
    let config = Config::default();
    assert!(!config.rbac_enabled());
    assert!(config.permissions_for("web:admin").allows_all());
}
