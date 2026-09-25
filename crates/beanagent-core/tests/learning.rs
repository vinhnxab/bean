//! Regression M15: reflection, draft persistence, review and activation gating.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use beanagent_core::{Channel, Incoming, MemoryStore, Router, RouterDeps, RouterError};
use beanagent_llm::FakeProvider;
use beanagent_security::CapWorkspace;
use beanagent_skills::{NewSkillDraft, SkillCatalog, SkillDraftKind, skill_tools};
use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use beanagent_types::{Config, LlmResponse, Outbound, RunId, ToolCall, ToolSpec};
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

    fn risk(&self, _args: &serde_json::Value) -> beanagent_types::Risk {
        beanagent_types::Risk::Safe
    }

    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        Ok("ok".into())
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
    let store = Arc::new(MemoryStore::new());
    let notifications = Arc::new(SentNotifications::default());
    let router = Arc::new(Router::new(RouterDeps {
        config,
        store,
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(responses)),
        audit: None,
        skills_index: catalog.index(),
        skills: Some(catalog.clone()),
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
                Some(beanagent_types::RunEvent::Final { .. }) => break,
                Some(beanagent_types::RunEvent::Error { message, .. }) => {
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

#[tokio::test]
async fn eligible_run_creates_reviewable_draft_but_does_not_activate_it() {
    let (_temp, router, catalog, notifications) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("đã hoàn tất"),
            proposal("release-checklist", "new"),
        ],
        5,
    )
    .await;
    run(&router, "làm checklist phát hành").await;
    let out = wait_notification(&notifications).await;
    assert!(matches!(
        out.action,
        Some(beanagent_types::OutboundAction::SkillDraft { .. })
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
    wait_notification(&notifications).await;
    run(&router, "lần hai").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let drafts = catalog.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].name, "first-skill");
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
        ],
        1,
    )
    .await;
    catalog
        .create("existing-skill", "Hướng dẫn cũ.", "# Old guide")
        .unwrap();
    catalog.reload().unwrap();

    run(&router, "nạp skill rồi làm khác hướng dẫn").await;
    wait_notification(&notifications).await;
    let draft = catalog.list_drafts().unwrap().remove(0);
    assert_eq!(draft.kind, beanagent_skills::SkillDraftKind::Update);
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
        ],
        5,
    )
    .await;
    run(&router, "tạo đề xuất").await;
    wait_notification(&notifications).await;
    let id = catalog.list_drafts().unwrap()[0].id.clone();
    assert!(matches!(
        router.approve_draft(&id, "test:khác").await,
        Err(RouterError::Forbidden(_))
    ));
    assert!(catalog.get("reviewed-skill").is_err());
}
