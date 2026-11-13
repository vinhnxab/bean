//! Regression M15: reflection, draft persistence, review and activation gating.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bean_core::{Channel, Incoming, MemoryStore, Router, RouterDeps, RouterError};
use bean_llm::{ChatRequest, FakeProvider, LlmError, LlmProvider};
use bean_security::{AuditLog, CapWorkspace};
use bean_skills::{NewSkillDraft, SkillCatalog, SkillDraftKind, skill_tools};
use bean_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use bean_types::{Config, LlmResponse, Outbound, RunId, ToolCall, ToolSpec};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
struct SentNotifications(Mutex<Vec<Outbound>>);

#[async_trait]
impl Channel for SentNotifications {
    fn name(&self) -> &'static str {
        "test"
    }

    async fn run(&self, _router: Arc<Router>, shutdown: CancellationToken) -> AnyResult<()> {
        shutdown.cancelled().await;
        Ok(())
    }

    async fn send(&self, _chat_id: &str, out: Outbound) -> AnyResult<()> {
        self.0.lock().unwrap().push(out);
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Probe;

#[async_trait]
impl Tool for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "probe",
            "tool đếm bước cho learning loop",
            serde_json::json!({"type": "object", "additionalProperties": false}),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> bean_types::Risk {
        bean_types::Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        Ok("ok".into())
    }
}

/// Ghi lại model được yêu cầu qua `chat_with_model` — dùng để kiểm chứng
/// `[llm].small_model` được dùng cho reflection.
#[derive(Debug)]
struct ModelRecordingProvider {
    inner: FakeProvider,
    models: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmProvider for ModelRecordingProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.inner.chat(req).await
    }

    async fn chat_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmResponse, LlmError> {
        self.models.lock().unwrap().push(model.to_string());
        self.inner.chat(req).await
    }

    fn name(&self) -> &'static str {
        "recording"
    }
}

fn proposal(name: &str, kind: &str) -> LlmResponse {
    LlmResponse::text_only(
        serde_json::json!({
            "reusable": true,
            "proposal": {
                "kind": kind,
                "name": name,
                "description": "Dùng khi cần lặp lại quy trình này.",
                "body": "# Reusable guide\n\n1. Kiểm tra đầu vào.\n2. Làm từng bước.",
                "reason": "Run thành công sau nhiều bước có thể lặp lại."
            }
        })
        .to_string(),
    )
}

fn tool_calls(count: usize) -> LlmResponse {
    LlmResponse::with_tool_calls(
        (0..count)
            .map(|index| ToolCall::new(format!("call-{index}"), "probe", serde_json::json!({})))
            .collect(),
    )
}

async fn fixture(
    responses: Vec<LlmResponse>,
    min_tool_calls: u32,
) -> (
    tempfile::TempDir,
    Arc<Router>,
    SkillCatalog,
    Arc<SentNotifications>,
) {
    fixture_with_llm(Arc::new(FakeProvider::new(responses)), min_tool_calls, None).await
}

async fn fixture_with_llm(
    llm: Arc<dyn LlmProvider>,
    min_tool_calls: u32,
    small_model: Option<String>,
) -> (
    tempfile::TempDir,
    Arc<Router>,
    SkillCatalog,
    Arc<SentNotifications>,
) {
    fixture_full(llm, min_tool_calls, small_model, None).await
}

/// Biến thể có audit log (K19) — các test cũ dùng [`fixture`] vẫn qua đường `None`.
async fn fixture_full(
    llm: Arc<dyn LlmProvider>,
    min_tool_calls: u32,
    small_model: Option<String>,
    audit: Option<Arc<AuditLog>>,
) -> (
    tempfile::TempDir,
    Arc<Router>,
    SkillCatalog,
    Arc<SentNotifications>,
) {
    let temp = tempfile::tempdir().unwrap();
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    registry.register(Arc::new(Probe)).unwrap();
    let skills_root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&skills_root),
        skills_root.clone(),
        skills_root.join("_drafts"),
    );
    for tool in skill_tools(catalog.clone()) {
        registry.register(tool).unwrap();
    }
    let mut config = Config::default();
    config.agent.max_steps = 4;
    config.agent.allowed_users.push("test:user".into());
    config.learning.min_tool_calls = min_tool_calls;
    config.learning.proposal_interval_minutes = 60;
    config.llm.small_model = small_model;
    let store = Arc::new(MemoryStore::new());
    let notifications = Arc::new(SentNotifications::default());
    let router = Arc::new(Router::new(RouterDeps {
        config_path: None,
        speech: None,
        default_profile: config.llm.as_profile(),
        config,
        store,
        registry: Arc::new(registry),
        llm,
        audit,
        skills_index: catalog.index(),
        skills: Some(catalog.clone()),
        llm_state_path: None,

        checkpoints: None,
    }));
    router.register_channel(notifications.clone()).unwrap();
    (temp, router, catalog, notifications)
}

async fn run(router: &Router, text: &str) -> RunId {
    let mut events = router.events();
    let run_id = router
        .submit(Incoming::new("test", "chat", "test:user", text))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match router.recv_event(&mut events).await {
                Some(bean_types::RunEvent::Final { .. }) => break,
                Some(bean_types::RunEvent::Error { message, .. }) => {
                    panic!("run lỗi: {message}")
                }
                Some(_) => {}
                None => panic!("Router đã đóng"),
            }
        }
    })
    .await
    .unwrap();
    run_id
}

async fn wait_notification(notifications: &SentNotifications) -> Outbound {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(out) = notifications.0.lock().unwrap().first().cloned() {
                return out;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

/// (K16) Chờ reflection lưu đề xuất ở trạng thái chờ (chưa phải draft) — điểm
/// đồng bộ xác định trước khi gửi lượt kế tiếp, tránh race promotion/relection.
async fn wait_pending(catalog: &SkillCatalog) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !catalog.pending_proposals().unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

/// (K16) Sau run chưa thăng hạng thì không được có notification nào.
async fn assert_no_notification(notifications: &SentNotifications) {
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        notifications.0.lock().unwrap().is_empty(),
        "chưa được có notification trước khi thăng hạng"
    );
}

#[tokio::test]
async fn eligible_run_creates_reviewable_draft_but_does_not_activate_it() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("đã hoàn tất"),
            proposal("release-checklist", "new"),
            LlmResponse::text_only("rất rõ ràng"), // lượt 2: agent trả lời trung tính
        ],
        5,
    )
    .await;
    run(&router, "làm checklist phát hành").await;
    wait_pending(&catalog).await;
    // (K16) Sau run 1: chỉ candidate — chưa có draft, chưa báo người dùng.
    assert_no_notification(&notifications).await;
    assert!(catalog.list_drafts().unwrap().is_empty());

    // Lượt kế tiếp trung tính ⇒ thăng hạng: draft + notification.
    run(&router, "tốt lắm, cảm ơn").await;
    let out = wait_notification(&notifications).await;
    assert!(matches!(
        out.action,
        Some(bean_types::OutboundAction::SkillDraft { .. })
    ));
    let drafts = catalog.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].name, "release-checklist");
    assert!(catalog.get("release-checklist").is_err());

    run(&router, &format!("/approve {}", drafts[0].id)).await;
    assert!(catalog.get("release-checklist").is_ok());
    assert!(catalog.list_drafts().unwrap().is_empty());
}

#[tokio::test]
async fn reflection_uses_small_model_when_configured() {
    let provider = Arc::new(ModelRecordingProvider {
        inner: FakeProvider::new(vec![
            tool_calls(5),
            LlmResponse::text_only("đã hoàn tất"),
            proposal("small-model-skill", "new"),
            LlmResponse::text_only("rất rõ ràng"), // lượt 2: agent trả lời trung tính
        ]),
        models: Mutex::new(vec![]),
    });
    let (_temp, router, catalog, notifications) =
        fixture_with_llm(provider.clone(), 5, Some("m-re".into())).await;
    run(&router, "làm checklist").await;
    wait_pending(&catalog).await;
    // Hai lượt đầu của run chính dùng model mặc định; lượt reflection (thứ ba)
    // phải dùng small_model. Block để guard không vượt ra ngoài (lint
    // `await_holding_lock` phân tích lexical scope).
    {
        let models = provider.models.lock().unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models.last().map(String::as_str), Some("m-re"));
    }

    run(&router, "cảm ơn, rõ ràng lắm").await;
    wait_notification(&notifications).await;
    assert_eq!(catalog.list_drafts().unwrap().len(), 1);
    // Lượt 2 chỉ là agent loop (thăng hạng không gọi LLM) — lại dùng model chính.
    {
        let models = provider.models.lock().unwrap();
        assert_eq!(models.len(), 4);
        assert_eq!(
            models.last().map(String::as_str),
            Some(Config::default().llm.model.as_str())
        );
    }

    // Không cấu hình small_model ⇒ reflection dùng model chính.
    let provider = Arc::new(ModelRecordingProvider {
        inner: FakeProvider::new(vec![tool_calls(5), LlmResponse::text_only("xong")]),
        models: Mutex::new(vec![]),
    });
    let (_temp, router, _catalog, _notifications) =
        fixture_with_llm(provider.clone(), 5, None).await;
    run(&router, "lần này không có proposal").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    // Reflection vẫn được thử (script cạn ⇒ lỗi best-effort) nhưng luôn dùng model chính.
    {
        let models = provider.models.lock().unwrap();
        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|m| m == &Config::default().llm.model));
    }
}

#[tokio::test]
async fn below_threshold_and_invalid_reflection_do_not_create_drafts() {
    let (_temp, router, catalog, _) = fixture(
        vec![
            tool_calls(4),
            LlmResponse::text_only("xong"),
            LlmResponse::text_only("{\"reusable\":true,\"proposal\":null}"),
        ],
        5,
    )
    .await;
    run(&router, "chưa đủ bước").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(catalog.list_drafts().unwrap().is_empty());

    let (_temp, router, catalog, _) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("xong"),
            LlmResponse::text_only("{\"reusable\":true,\"proposal\":{\"kind\":\"new\"}}"),
        ],
        5,
    )
    .await;
    run(&router, "reflection sai schema").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
}

#[tokio::test]
async fn proposal_rate_limit_prevents_second_reflection() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("lần một"),
            proposal("first-skill", "new"),
            tool_calls(5),
            LlmResponse::text_only("lần hai"),
            proposal("second-skill", "new"),
        ],
        5,
    )
    .await;
    run(&router, "lần một").await;
    wait_pending(&catalog).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
    // Lượt 2 vừa thăng hạng candidate của lượt 1 vừa đủ điều kiện reflection —
    // nhưng cooldown đã tick lúc reflection lượt 1 nên reflection lượt 2 bị chặn.
    run(&router, "lần hai").await;
    wait_notification(&notifications).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let drafts = catalog.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].name, "first-skill");
    assert!(catalog.pending_proposals().unwrap().is_empty());
}

#[tokio::test]
async fn disabled_learning_never_calls_reflection() {
    let temp = tempfile::tempdir().unwrap();
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    registry.register(Arc::new(Probe)).unwrap();
    let skills_root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&skills_root),
        skills_root.clone(),
        skills_root.join("_drafts"),
    );
    let mut config = Config::default();
    config.agent.max_steps = 3;
    config.agent.allowed_users.push("test:user".into());
    config.learning.enabled = false;
    let router = Router::new(RouterDeps {
        config_path: None,
        speech: None,
        default_profile: config.llm.as_profile(),
        config,
        store: Arc::new(MemoryStore::new()),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(vec![
            tool_calls(5),
            LlmResponse::text_only("xong"),
            proposal("must-not-exist", "new"),
        ])),
        audit: None,
        skills_index: String::new(),
        skills: Some(catalog.clone()),
        llm_state_path: None,

        checkpoints: None,
    });
    run(&router, "tắt learning").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
}

#[tokio::test]
async fn update_proposal_is_diff_and_only_loaded_skill_can_be_updated() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            LlmResponse::with_tool_calls(vec![
                ToolCall::new(
                    "load",
                    "load_skill",
                    serde_json::json!({"name": "existing-skill"}),
                ),
                ToolCall::new("p1", "probe", serde_json::json!({})),
                ToolCall::new("p2", "probe", serde_json::json!({})),
                ToolCall::new("p3", "probe", serde_json::json!({})),
                ToolCall::new("p4", "probe", serde_json::json!({})),
            ]),
            LlmResponse::text_only("làm theo cách tốt hơn"),
            proposal("existing-skill", "update"),
            LlmResponse::text_only("ok"), // lượt 2: agent trả lời trung tính
        ],
        1,
    )
    .await;
    catalog
        .create("existing-skill", "Hướng dẫn cũ.", "# Old guide")
        .unwrap();
    catalog.reload().unwrap();

    run(&router, "nạp skill rồi làm khác hướng dẫn").await;
    wait_pending(&catalog).await;
    run(&router, "cảm ơn, đúng ý tôi").await;
    wait_notification(&notifications).await;
    let draft = catalog.list_drafts().unwrap().remove(0);
    assert_eq!(draft.kind, bean_skills::SkillDraftKind::Update);
    assert!(draft.content.contains("--- a/SKILL.md"));
    assert!(draft.content.contains("+# Reusable guide"));
    assert!(
        catalog
            .get("existing-skill")
            .unwrap()
            .content
            .contains("# Old guide")
    );
    router.approve_draft(&draft.id, "test:user").await.unwrap();
    assert!(
        catalog
            .get("existing-skill")
            .unwrap()
            .content
            .contains("# Reusable guide")
    );
}

#[tokio::test]
async fn reject_slash_command_removes_draft_without_activation() {
    let (_temp, router, catalog, _) = fixture(vec![], 5).await;
    let draft = catalog
        .create_draft(NewSkillDraft {
            name: "rejected-skill".into(),
            kind: SkillDraftKind::New,
            description: "Mô tả hợp lệ.".into(),
            body: "# Guide".into(),
            reason: "Quy trình có thể tái sử dụng.".into(),
            source_session_id: 1,
            source_channel: "test".into(),
            source_chat_id: "chat".into(),
            created_at: "2026-09-25T10:00:00Z".into(),
        })
        .unwrap();
    run(&router, &format!("/reject {}", draft.id)).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
    assert!(catalog.get("rejected-skill").is_err());
}

#[tokio::test]
async fn approval_rejects_actor_outside_allowlist() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("xong"),
            proposal("reviewed-skill", "new"),
            LlmResponse::text_only("ok"), // lượt 2: agent trả lời trung tính
        ],
        5,
    )
    .await;
    run(&router, "tạo đề xuất").await;
    wait_pending(&catalog).await;
    run(&router, "cảm ơn nhé").await;
    wait_notification(&notifications).await;
    let id = catalog.list_drafts().unwrap()[0].id.clone();
    assert!(matches!(
        router.approve_draft(&id, "test:khác").await,
        Err(RouterError::Forbidden(_))
    ));
    assert!(catalog.get("reviewed-skill").is_err());
}

/// K19: quyết định duyệt/bỏ draft (kể cả lần thử bị từ chối quyền hay draft không
/// tồn tại) phải còn dấu vết bền trong audit JSONL với actor, thời điểm và draft nào.
#[tokio::test]
async fn draft_decisions_are_written_to_audit_log() {
    let temp = tempfile::tempdir().unwrap();
    let audit = Arc::new(AuditLog::open(&temp.path().join("audit")).unwrap());
    let (_t, router, catalog, _notifications) = fixture_full(
        Arc::new(FakeProvider::new(vec![])),
        5,
        None,
        Some(audit.clone()),
    )
    .await;

    let make = |name: &str| NewSkillDraft {
        name: name.into(),
        kind: SkillDraftKind::New,
        description: "Mô tả hợp lệ.".into(),
        body: "# Guide".into(),
        reason: "Quy trình có thể tái sử dụng.".into(),
        source_session_id: 1,
        source_channel: "test".into(),
        source_chat_id: "chat".into(),
        created_at: "2026-11-12T10:00:00Z".into(),
    };

    let approved = catalog.create_draft(make("approved-skill")).unwrap();
    router
        .approve_draft(&approved.id, "test:user")
        .await
        .unwrap();

    let rejected = catalog.create_draft(make("rejected-skill")).unwrap();
    router
        .reject_draft(&rejected.id, "web:admin")
        .await
        .unwrap();

    // Lần thử không thành công vẫn phải còn dấu vết: draft không tồn tại và actor
    // ngoài allowlist.
    router
        .approve_draft("khong-ton-tai", "test:user")
        .await
        .unwrap_err();
    router
        .reject_draft(&approved.id, "test:khác")
        .await
        .unwrap_err();

    let lines = audit.read_recent(None, 10);
    assert_eq!(
        lines.len(),
        4,
        "mỗi quyết định/thử quyết định một dòng audit"
    );
    // `read_recent` trả mới trước: [reject bị cấm, approve lỗi, reject ok, approve ok].
    let reject_forbidden = &lines[0];
    assert_eq!(reject_forbidden["tool"], "skill_draft_reject");
    assert_eq!(reject_forbidden["decided_by"], "test:khác");
    assert_eq!(reject_forbidden["ok"], false);
    assert_eq!(reject_forbidden["channel"], "test");
    assert!(reject_forbidden["error"].as_str().is_some());

    let approve_missing = &lines[1];
    assert_eq!(approve_missing["tool"], "skill_draft_approve");
    assert_eq!(approve_missing["decided_by"], "test:user");
    assert_eq!(approve_missing["ok"], false);

    let reject_ok = &lines[2];
    assert_eq!(reject_ok["tool"], "skill_draft_reject");
    assert_eq!(reject_ok["decision"], "deny");
    assert_eq!(reject_ok["decided_by"], "web:admin");
    assert_eq!(reject_ok["channel"], "web");
    assert_eq!(reject_ok["ok"], true);
    assert_eq!(reject_ok["args"]["draft_id"], rejected.id);
    assert_eq!(reject_ok["args"]["name"], "rejected-skill");

    let approve_ok = &lines[3];
    assert_eq!(approve_ok["tool"], "skill_draft_approve");
    assert_eq!(approve_ok["decision"], "allow");
    assert_eq!(approve_ok["decided_by"], "test:user");
    assert_eq!(approve_ok["ok"], true);
    assert_eq!(approve_ok["args"]["draft_id"], approved.id);
    assert_eq!(approve_ok["args"]["name"], "approved-skill");
    assert_eq!(approve_ok["args"]["status"], "approved");
    assert!(approve_ok["ts"].as_str().is_some(), "phải có thời điểm");
}

/// (K16) Lượt kế tiếp mang phản hồi tiêu cực ⇒ candidate bị bỏ vĩnh viễn: lượt 3
/// trung tính cũng không phục hồi được — không draft, không notification.
#[tokio::test]
async fn complaint_discards_candidate_permanently() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("xong"),
            proposal("discarded-skill", "new"),
            LlmResponse::text_only("xin lỗi, để làm lại"),
            LlmResponse::text_only("đã sửa lại"),
        ],
        5,
    )
    .await;
    run(&router, "làm checklist").await;
    wait_pending(&catalog).await;

    run(&router, "sai rồi, làm lại đi").await;
    assert_no_notification(&notifications).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
    assert!(catalog.pending_proposals().unwrap().is_empty());

    run(&router, "ok giờ tốt rồi").await;
    assert_no_notification(&notifications).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
    assert!(catalog.pending_proposals().unwrap().is_empty());
}

/// (K16) Candidate nằm trong state file của catalog: dựng lại Router trên cùng
/// thư mục dữ liệu (tương đương restart tiến trình) vẫn thăng hạng được.
#[tokio::test]
async fn candidate_survives_router_rebuild() {
    let temp = tempfile::tempdir().unwrap();
    let workspace_path = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    registry.register(Arc::new(Probe)).unwrap();
    let skills_root = temp.path().join("skills");
    let catalog = SkillCatalog::load_with_paths(
        std::slice::from_ref(&skills_root),
        skills_root.clone(),
        skills_root.join("_drafts"),
    );
    let mut config = Config::default();
    config.agent.max_steps = 4;
    config.agent.allowed_users.push("test:user".into());
    config.learning.min_tool_calls = 5;
    config.learning.proposal_interval_minutes = 60;
    let notifications = Arc::new(SentNotifications::default());

    let first = Router::new(RouterDeps {
        config_path: None,
        speech: None,
        default_profile: config.llm.as_profile(),
        config: config.clone(),
        store: Arc::new(MemoryStore::new()),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(vec![
            tool_calls(5),
            LlmResponse::text_only("xong"),
            proposal("restart-skill", "new"),
        ])),
        audit: None,
        skills_index: catalog.index(),
        skills: Some(catalog.clone()),
        llm_state_path: None,
        checkpoints: None,
    });
    first.register_channel(notifications.clone()).unwrap();
    run(&first, "làm checklist").await;
    wait_pending(&catalog).await;
    drop(first);

    // "Restart": Router thứ hai trên cùng temp paths — catalog dựng lại từ đĩa
    // phải đọc lại candidate, store mới vẫn chọn phiên có cùng id (MemoryStore).
    let workspace = Arc::new(CapWorkspace::open(temp.path().join("workspace")).unwrap());
    let mut registry = ToolRegistry::with_workspace(workspace);
    registry.register(Arc::new(Probe)).unwrap();
    let restarted = Router::new(RouterDeps {
        config_path: None,
        speech: None,
        default_profile: config.llm.as_profile(),
        config,
        store: Arc::new(MemoryStore::new()),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(vec![LlmResponse::text_only(
            "rất vui khi giúp được",
        )])),
        audit: None,
        skills_index: catalog.index(),
        skills: Some(catalog.clone()),
        llm_state_path: None,
        checkpoints: None,
    });
    restarted.register_channel(notifications.clone()).unwrap();
    run(&restarted, "tốt lắm, cảm ơn").await;
    wait_notification(&notifications).await;
    let drafts = catalog.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].name, "restart-skill");
}

/// (K16) Slash command không phải lượt agent ⇒ không thăng hạng candidate; nó
/// vẫn nằm chờ lượt agent kế tiếp.
#[tokio::test]
async fn slash_command_does_not_promote_candidate() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("xong"),
            proposal("held-skill", "new"),
        ],
        5,
    )
    .await;
    run(&router, "làm checklist").await;
    wait_pending(&catalog).await;

    run(&router, "/memory").await;
    assert_no_notification(&notifications).await;
    assert!(catalog.list_drafts().unwrap().is_empty());
    assert_eq!(catalog.pending_proposals().unwrap().len(), 1);
}
