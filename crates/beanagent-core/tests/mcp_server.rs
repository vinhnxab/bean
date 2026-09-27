//! M25 — Bean làm **MCP server read-only** (`Plan.md` mục 5, milestone M25).
//!
//! Bằng chứng cho **4 test bắt buộc** của milestone:
//!
//! 1. `client_without_valid_token_is_rejected_before_handshake` — client không token hợp
//!    lệ bị từ chối **trước khi** thấy bất kỳ tool nào, và cả trước khi tạo được handler.
//! 2. `finance_client_sees_only_billing_tools` — role `finance-readonly` chỉ thấy đúng
//!    tool `billing-read` trong `tools/list`.
//! 3. `calling_a_non_exposed_tool_directly_is_denied` — gọi thẳng tên tool **không** thuộc
//!    bề mặt MCP (ví dụ `dev_write`) bị từ chối ở tầng thực thi, không chỉ ẩn khỏi danh sách.
//! 4. `client_arguments_are_sanitised_before_reaching_the_tool` — tham số chứa chuỗi giống
//!    SQL/FTS injection bị làm sạch, không lọt xuống tầng dưới nguyên văn.
//!
//! Ngoài ra có test cho cổng expose: role `admin` (tag `*`) **không** vượt được phạm vi
//! cứng của M25.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use beanagent_core::mcp_server::auth::McpAuth;
use beanagent_core::mcp_server::{self, ExposeDecision, expose_gate};
use beanagent_core::{Router, RouterDeps};
use beanagent_llm::FakeProvider;
use beanagent_memory::{MemoryStore, Store};
use beanagent_security::CapWorkspace;
use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use beanagent_types::config::{McpClientConfig, RoleConfig};
use beanagent_types::{Config, Risk, RolePermissions, ToolSpec};
use tempfile::TempDir;

/// Tool ghi lại **tham số thật sự nhận được** để chứng minh việc làm sạch có hiệu lực.
#[derive(Debug)]
struct EchoArgs {
    name: &'static str,
    risk: Risk,
    tags: Vec<&'static str>,
    seen: Mutex<Vec<serde_json::Value>>,
}

#[async_trait]
impl Tool for EchoArgs {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "tool ghi lại tham số",
            serde_json::json!({"type": "object"}),
        )
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        self.risk
    }

    fn required_tags(&self) -> Vec<&str> {
        self.tags.clone()
    }

    async fn call(&self, _ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        self.seen.lock().unwrap().push(args.clone());
        Ok(args.to_string())
    }
}

fn role(name: &str, tags: &[&str]) -> RoleConfig {
    RoleConfig {
        name: name.into(),
        tool_tags: tags.iter().map(|s| (*s).to_string()).collect(),
        allowed_tool_tags: vec![],
        forbid_tags: vec![],
        context_budget_tokens: None,
        daily_token_budget: None,
    }
}

/// Cấu hình + registry + Router dùng chung cho các test.
///
/// Registry cố tình có **cả** tool ghi (`dev_write`, `Confirm`) lẫn tool không tag, để
/// chứng minh cổng expose chặn đúng ở tầng tên gọi chứ không "may mắn" không có sẵn.
fn setup() -> (TempDir, Arc<Router>, Config, Arc<EchoArgs>) {
    let dir = TempDir::new().unwrap();
    let mut config = Config::default();
    config.agent.workspace = dir.path().to_path_buf();
    config.roles = vec![
        role("admin", &["*"]),
        role("finance-readonly", &["billing-read"]),
        role("monitor", &["infra-read"]),
    ];
    config.mcp_clients = vec![
        McpClientConfig {
            name: "cline".into(),
            role: "finance-readonly".into(),
        },
        McpClientConfig {
            name: "ops".into(),
            role: "monitor".into(),
        },
    ];
    config.agent.user_roles = [
        (
            "mcp-client:cline".to_string(),
            "finance-readonly".to_string(),
        ),
        ("mcp-client:ops".to_string(), "monitor".to_string()),
    ]
    .into_iter()
    .collect();
    config.mcp_server.enabled = true;
    config.validate().expect("cấu hình M25 phải hợp lệ");

    let ws = Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap());
    let mut registry = ToolRegistry::with_workspace(ws);
    let billing = Arc::new(EchoArgs {
        name: "billing_read_cost",
        risk: Risk::Safe,
        tags: vec!["billing-read"],
        seen: Mutex::new(Vec::new()),
    });
    let infra = Arc::new(EchoArgs {
        name: "infra_read_logs",
        risk: Risk::Safe,
        tags: vec!["infra-read"],
        seen: Mutex::new(Vec::new()),
    });
    let dev = Arc::new(EchoArgs {
        name: "dev_write",
        risk: Risk::Confirm,
        tags: vec!["dev-write"],
        seen: Mutex::new(Vec::new()),
    });
    registry.register(billing.clone()).unwrap();
    registry.register(infra).unwrap();
    registry.register(dev).unwrap();

    let store = Arc::new(MemoryStore::default());
    let store_dyn: Arc<dyn Store> = store;
    let router = Arc::new(Router::new(RouterDeps {
        config: config.clone(),
        store: store_dyn,
        registry: Arc::new(registry),
        llm: Arc::new(FakeProvider::echo()),
        audit: None,
        skills_index: String::new(),
        skills: None,
    }));
    (dir, router, config, billing)
}

/// Test 1: token sai/không hợp lệ ⇒ từ chối, trước khi client thấy tool nào.
#[tokio::test]
async fn client_without_valid_token_is_rejected_before_handshake() {
    let (_dir, _router, config, _billing) = setup();
    let store = Arc::new(MemoryStore::default());
    let good = mcp_server::new_token().unwrap();
    store
        .create_mcp_client(
            &mcp_server::hash_token(&good),
            "cline",
            "finance-readonly",
            &chrono::Utc::now().to_rfc3339(),
            "",
        )
        .await
        .unwrap();
    let auth = McpAuth::new(store);

    // Token đúng thì vào được, và identity là `mcp-client:<name>` — tách biệt hoàn toàn
    // khỏi `telegram:`/`web:` (Plan.md M25 mục 2).
    let identity = auth.authenticate(&config, &good).await.unwrap();
    assert_eq!(identity.user_id, "mcp-client:cline");
    assert_eq!(identity.role, "finance-readonly");

    // Token sai, rỗng, và chuỗi tùy ý đều bị chặn.
    let other = mcp_server::new_token().unwrap();
    assert!(auth.authenticate(&config, &other).await.is_err());
    assert!(auth.authenticate(&config, "").await.is_err());
    assert!(
        auth.authenticate(&config, "../../etc/passwd")
            .await
            .is_err()
    );

    // Token đúng nhưng client đã bị gỡ khỏi cấu hình ⇒ cũng từ chối (lỗi cấu hình, log rõ).
    let mut no_client = config.clone();
    no_client.mcp_clients.clear();
    assert!(auth.authenticate(&no_client, &good).await.is_err());

    // Token hết hạn cũng bị từ chối.
    let expired = Arc::new(MemoryStore::default());
    let stale = mcp_server::new_token().unwrap();
    expired
        .create_mcp_client(
            &mcp_server::hash_token(&stale),
            "cline",
            "finance-readonly",
            "2000-01-01T00:00:00+00:00",
            "2000-01-02T00:00:00+00:00",
        )
        .await
        .unwrap();
    let auth2 = McpAuth::new(expired);
    assert!(auth2.authenticate(&config, &stale).await.is_err());
}

/// Test 2: role `finance-readonly` chỉ thấy đúng tool `billing-read` trong `tools/list`.
#[tokio::test]
async fn finance_client_sees_only_billing_tools() {
    let (_dir, router, _config, _billing) = setup();
    let perms = router.mcp_permissions("mcp-client:cline").unwrap();
    assert_eq!(perms.role, "finance-readonly");
    assert_eq!(
        mcp_server::gate::visible_names(router.registry(), &perms),
        vec!["billing_read_cost".to_string()]
    );

    // Client khác (role monitor) thấy bề mặt khác — không dùng chung danh sách.
    let ops = router.mcp_permissions("mcp-client:ops").unwrap();
    assert_eq!(
        mcp_server::gate::visible_names(router.registry(), &ops),
        vec!["infra_read_logs".to_string()]
    );

    // Client không có trong `user_roles` là `no-access` ⇒ không thấy gì.
    let ghost = router.mcp_permissions("mcp-client:khong-ton-tai").unwrap();
    assert!(mcp_server::gate::visible_names(router.registry(), &ghost).is_empty());
}

/// Test 3: gọi thẳng tool không thuộc bề mặt MCP bị **từ chối**, không chỉ bị ẩn.
#[tokio::test]
async fn calling_a_non_exposed_tool_directly_is_denied() {
    let (_dir, router, _config, _billing) = setup();
    // `dev_write` không có trong `tools/list` của client finance…
    let perms = router.mcp_permissions("mcp-client:cline").unwrap();
    assert_eq!(
        expose_gate("dev_write", router.registry(), &perms),
        ExposeDecision::TagNotExposed
    );
    // …nhưng nếu client gọi thẳng tên thì phải bị chặn ở **tầng thực thi**.
    let denied = router
        .call_tool_as(
            "mcp-client:cline",
            "dev_write",
            serde_json::json!({"path": "src/x.rs"}),
        )
        .await;
    assert!(
        matches!(&denied, Err(beanagent_core::RouterError::ToolNotExposed(name)) if name == "dev_write"),
        "gọi thẳng tool ngoài phạm vi phải bị từ chối, không phải chạy: {denied:?}"
    );
    // Tool không tồn tại cũng phải bị chặn, không panic và không rơi xuống tầng dưới.
    assert!(
        router
            .call_tool_as("mcp-client:cline", "khong_ton_tai", serde_json::json!({}))
            .await
            .is_err()
    );
    // Client `no-access` không gọi được gì, kể cả tool an toàn.
    assert!(
        router
            .call_tool_as(
                "mcp-client:khong-ton-tai",
                "billing_read_cost",
                serde_json::json!({})
            )
            .await
            .is_err()
    );
}
/// Cổng expose phải chặn được cả role `admin` (tag `*`) — phạm vi cứng của M25.
#[test]
fn admin_wildcard_cannot_reach_write_tools_through_mcp() {
    let (_dir, router, _config, _billing) = setup();
    let admin = RolePermissions::unrestricted("admin");
    // Theo RBAC, `admin` thấy *mọi* tool…
    assert!(router.registry().allows("dev_write", &admin));
    // …nhưng cổng expose của MCP server vẫn chặn.
    assert_eq!(
        expose_gate("dev_write", router.registry(), &admin),
        ExposeDecision::TagNotExposed
    );
    assert_eq!(
        mcp_server::gate::visible_names(router.registry(), &admin),
        vec![
            "billing_read_cost".to_string(),
            "infra_read_logs".to_string()
        ]
    );
}

/// Test 4: tham số từ client chứa chuỗi giống SQL/FTS injection bị làm sạch.
#[tokio::test]
async fn client_arguments_are_sanitised_before_reaching_the_tool() {
    let (_dir, router, _config, billing) = setup();
    let injection = "'; DROP TABLE memories; --\u{0}\" OR 1=1 OR \"";
    let args = serde_json::json!({ "query": injection, "period": "30d" });
    let out = router
        .call_tool_as("mcp-client:cline", "billing_read_cost", args)
        .await
        .expect("tool được expose thì phải chạy được");
    {
        let seen = billing.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        // Ký tự điều khiển bị bỏ trước khi chạm tầng dưới (không lọt nguyên văn).
        assert!(!seen[0].to_string().contains('\u{0}'));
    }
    // Nội dung người dùng vẫn còn: ta chỉ vô hiệu hoá ký tự điều khiển và cắt độ dài,
    // không "tự ý" sửa dữ liệu — truy vấn FTS còn được làm sạch thêm ở đúng chỗ dùng nó.
    assert!(out.contains("DROP TABLE"));

    // Tham số vượt trần độ sâu bị cắt, không làm treo hay lỗi.
    let mut deep = serde_json::json!("x");
    for _ in 0..64 {
        deep = serde_json::json!({ "a": deep });
    }
    router
        .call_tool_as("mcp-client:cline", "billing_read_cost", deep)
        .await
        .expect("thành công với tham số quá sâu");
    assert!(
        billing
            .seen
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .to_string()
            .contains("quá sâu")
    );
}

/// `tools/list` phải ổn định (sort theo tên) để client cache không bị lệch giữa hai lần gọi.
#[test]
fn exposed_tool_list_is_sorted_and_stable() {
    let (_dir, router, _config, _billing) = setup();
    let admin = RolePermissions::unrestricted("admin");
    let first = mcp_server::gate::visible_names(router.registry(), &admin);
    let second = mcp_server::gate::visible_names(router.registry(), &admin);
    assert_eq!(first, second);
    let mut sorted = first.clone();
    sorted.sort();
    assert_eq!(first, sorted);
}
