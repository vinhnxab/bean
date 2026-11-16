//! K20: draft phát sinh từ scheduler phải về đúng tay **chủ session** (có nút inline Telegram).
//!
//! # Vì sao đây là test "đặc ta", không phải sửa code
//!
//! K20 được ghi khi còn nghi ngờ "scheduler không gắn với user Telegram cụ thể nên
//! adapter gửi draft dạng text". Kiểm chứng lại code cho thấy hành vi hiện tại đã
//! đúng thiết kế phương án (b) của K20 mà không cần cấu hình mới:
//!
//! 1. Scheduler **từ M13 gắn với session**: `Scheduler::process_task` đọc
//!    `session_info(session)` (từ chối khi `user_id` rỗng/kênh lệch) rồi
//!    `submit_scheduled(Incoming::new(channel, chat_id, info.user_id, prompt))`.
//! 2. Run nền không bị chặn reflection: `router/run.rs::execute` spawn reflection
//!    cho mọi run kết thúc `Final` đủ `min_tool_calls` — kể cả scheduler.
//! 3. (K16/D28) Reflection chỉ lưu **candidate**; lượt kế tiếp **cùng phiên** bắt đầu
//!    thì `resolve_pending_promotion` thăng hạng và `notify_skill_draft` gửi thông
//!    báo về kênh/chat của lượt với `OutboundAction::SkillDraft.actor` = chủ session.
//! 4. Adapter Telegram gắn nút inline Duyệt/Bỏ khi `actor.starts_with("telegram:")`
//!    và callback so khớp `actor` ⇒ chỉ owner bấm được; `authorize` của Router vẫn
//!    kiểm tra lại `allowed_users` khi nhận quyết định.
//!
//! Test này khoá toàn bộ chuỗi đó bằng FakeProvider + FixedClock, không mạng:
//! task định kỳ chạy trên session `(telegram, 100, telegram:42)`, run 1 sinh
//! candidate, run 2 (định kỳ) thăng hạng — notification phải mang actor chủ session
//! và gửi về đúng chat, không phải user/chat trừu tượng nào khác.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bean_core::{Channel, FixedClock, MemoryStore, Router, RouterDeps, Scheduler};
use bean_llm::FakeProvider;
use bean_memory::NewScheduledTask;
use bean_memory::traits::*;
use bean_security::CapWorkspace;
use bean_skills::{SkillCatalog, skill_tools};
use bean_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use bean_types::{Config, LlmResponse, Outbound, OutboundAction, Risk, ToolCall, ToolSpec};
use chrono::{DateTime, TimeZone, Utc};
use tokio_util::sync::CancellationToken;

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("thời điểm test hợp lệ")
}

/// Kênh "telegram" ghi lại `(chat_id, Outbound)` — dùng để khẳng định draft từ
/// scheduler về đúng chat của phiên và mang actor chủ session.
#[derive(Debug, Default)]
struct SentNotifications(Mutex<Vec<(String, Outbound)>>);

#[async_trait]
impl Channel for SentNotifications {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn run(&self, _router: Arc<Router>, shutdown: CancellationToken) -> AnyResult<()> {
        shutdown.cancelled().await;
        Ok(())
    }

    async fn send(&self, chat_id: &str, out: Outbound) -> AnyResult<()> {
        self.0.lock().unwrap().push((chat_id.to_string(), out));
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

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
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
    now: DateTime<Utc>,
) -> (
    tempfile::TempDir,
    Arc<MemoryStore>,
    Arc<Router>,
    Arc<FixedClock>,
    Arc<SentNotifications>,
    SkillCatalog,
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
    config.agent.timezone = "Asia/Ho_Chi_Minh".into();
    config.agent.allowed_users.push("telegram:42".into());
    config.learning.min_tool_calls = 5;
    config.learning.proposal_interval_minutes = 60;
    let store = Arc::new(MemoryStore::new());
    let notifications = Arc::new(SentNotifications::default());
    let router = Arc::new(Router::new(RouterDeps {
        config_path: None,
        speech: None,
        default_profile: config.llm.as_profile(),
        config,
        store: store.clone(),
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::new(responses)),
        audit: None,
        skills_index: catalog.index(),
        skills: Some(catalog.clone()),
        llm_state_path: None,

        checkpoints: None,
    }));
    router.register_channel(notifications.clone()).unwrap();
    let clock = Arc::new(FixedClock::new(now));
    (temp, store, router, clock, notifications, catalog)
}

fn task_input(session: bean_types::SessionId, now: DateTime<Utc>) -> NewScheduledTask {
    NewScheduledTask {
        cron: "0 * * * *".into(),
        prompt: "chạy kiểm tra định kỳ".into(),
        session_id: Some(session),
        channel: "telegram".into(),
        chat_id: "100".into(),
        allowed_tools: vec![],
        next_run: now.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        enabled: true,
    }
}

/// (K16) Chờ reflection lưu đề xuất ở trạng thái chờ — điểm đồng bộ xác định trước
/// khi chạy lượt kế tiếp, tránh race thăng hạng trước khi candidate kịp lưu.
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

async fn draft_notifications(channel: &SentNotifications) -> Vec<Outbound> {
    channel
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, out)| matches!(out.action, Some(OutboundAction::SkillDraft { .. })))
        .cloned()
        .map(|(_, out)| out)
        .collect()
}

/// Chờ tối đa 3s cho một notification SkillDraft (thăng hạng diễn ra trong run,
/// trước `Final` nên khi `run_once` trả về thì notification hẳn đã gửi).
async fn wait_draft_notification(channel: &SentNotifications) -> Outbound {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let drafts = draft_notifications(channel).await;
            if let Some(out) = drafts.first().cloned() {
                return out;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

/// (K16) Sau run chưa thăng hạng thì không được có notification SkillDraft nào
/// (notification Final của scheduler có `action: None`, không đếm).
async fn assert_no_draft_notification(channel: &SentNotifications) {
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        draft_notifications(channel).await.is_empty(),
        "chưa được có notification SkillDraft trước khi thăng hạng"
    );
}

#[tokio::test]
async fn scheduler_run_promotes_pending_proposal_and_notifies_session_owner() {
    let now = at(2026, 11, 16, 0, 0);
    let (_temp, store, router, clock, channel, catalog) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("đã hoàn tất"),
            proposal("scheduled-skill", "new"),
            LlmResponse::text_only("rất rõ ràng"),
        ],
        now,
    )
    .await;

    // Session Telegram thật: chủ phiên là `telegram:42` — đúng dạng user mà
    // adapter gắn nút inline Duyệt/Bỏ (`actor.starts_with("telegram:")`).
    let session = store
        .ensure_session_for_user("telegram", "100", "telegram:42", "")
        .await
        .unwrap();
    store.create_task(task_input(session, now)).await.unwrap();
    let scheduler = Scheduler::with_clock(
        store.clone(),
        router.clone(),
        "Asia/Ho_Chi_Minh",
        clock.clone(),
    )
    .unwrap();

    // Run 1 (định kỳ): 5 tool call + final ⇒ reflection lưu candidate, chưa draft.
    assert_eq!(scheduler.run_once().await.unwrap(), 1);
    wait_pending(&catalog).await;
    assert_no_draft_notification(&channel).await;
    assert!(catalog.list_drafts().unwrap().is_empty());

    // Run 2 (định kỳ, cùng phiên, prompt trung tính): thăng hạng ngay đầu lượt.
    clock.set(now + chrono::Duration::hours(1));
    assert_eq!(scheduler.run_once().await.unwrap(), 1);
    let out = wait_draft_notification(&channel).await;
    let (draft_id, draft_name, actor) = match out.action {
        Some(OutboundAction::SkillDraft { id, name, actor }) => (id, name, actor),
        other => panic!("notification không phải SkillDraft: {other:?}"),
    };
    // Điểm K20: actor là **chủ session** — adapter gắn nút inline cho đúng người
    // này, callback so khớp actor nên chỉ owner duyệt/bỏ được.
    assert_eq!(actor, "telegram:42");
    assert_eq!(draft_name, "scheduled-skill");
    assert!(out.text.contains("Đề xuất skill"));
    // Về đúng phiên và đúng chat của phiên (không phải chat trừu tượng nào khác).
    assert_eq!(out.session_id, session);

    // Draft chờ duyệt, chưa kích hoạt; quyết định vẫn qua đường chuẩn.
    let drafts = catalog.list_drafts().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].id, draft_id);
    assert!(catalog.get("scheduled-skill").is_err());
}

#[tokio::test]
async fn scheduled_prompt_complaint_discards_pending_proposal_without_draft() {
    let now = at(2026, 11, 16, 0, 0);
    // Prompt của task mang phản hồi tiêu cực ⇒ K16 phải bỏ candidate, dù run đó
    // đến từ scheduler chứ không phải người dùng gõ trực tiếp.
    let (_temp, store, router, clock, channel, catalog) = fixture(
        vec![
            tool_calls(5),
            LlmResponse::text_only("đã hoàn tất"),
            proposal("complained-skill", "new"),
            LlmResponse::text_only("rất rõ ràng"),
        ],
        now,
    )
    .await;
    let session = store
        .ensure_session_for_user("telegram", "100", "telegram:42", "")
        .await
        .unwrap();
    let mut task = task_input(session, now);
    task.prompt = "sai rồi, làm lại đi".into();
    store.create_task(task).await.unwrap();
    let scheduler =
        Scheduler::with_clock(store, router, "Asia/Ho_Chi_Minh", clock.clone()).unwrap();

    assert_eq!(scheduler.run_once().await.unwrap(), 1);
    wait_pending(&catalog).await;
    clock.set(now + chrono::Duration::hours(1));
    assert_eq!(scheduler.run_once().await.unwrap(), 1);

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(draft_notifications(&channel).await.is_empty());
    assert!(catalog.pending_proposals().unwrap().is_empty());
    assert!(catalog.list_drafts().unwrap().is_empty());
}
