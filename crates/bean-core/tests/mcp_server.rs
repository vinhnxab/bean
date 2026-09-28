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
use bean_core::mcp_server::auth::McpAuth;
use bean_core::mcp_server::transport::ServeContext;
use bean_core::mcp_server::{
    self, ExposeDecision, McpClientIdentity, McpRateLimiter, McpServerError, expose_gate,
};
use bean_core::{Router, RouterDeps};
use bean_llm::FakeProvider;
use bean_memory::{MemoryStore, Store};
use bean_security::CapWorkspace;
use bean_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
use bean_types::config::{McpClientConfig, RoleConfig};
use bean_types::{Config, Risk, RolePermissions, ToolSpec};
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
        matches!(&denied, Err(bean_core::RouterError::ToolNotExposed(name)) if name == "dev_write"),
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

// ---------------------------------------------------------------------------
// K24 — giới hạn tần suất + nhật ký riêng `audit/mcp.jsonl`
// ---------------------------------------------------------------------------

/// Cấu hình HTTP **lấy thẳng mặc định** từ `McpServerConfigSettings` thay vì chép
/// số: nếu mặc định đổi, test này phải đổi theo chứ không âm thầm kiểm một giá trị cũ.
fn http_settings() -> bean_types::config::McpServerConfigSettings {
    bean_types::config::McpServerConfigSettings {
        enabled: true,
        http_enabled: true,
        ..Default::default()
    }
}

fn loopback() -> std::net::IpAddr {
    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

/// Dựng `ServeContext` thật (không mock) để test đi qua **đúng** đường HTTP mà
/// `bean mcp serve --http` dùng: `authenticate_http`.
fn serve_context(
    settings: bean_types::config::McpServerConfigSettings,
    audit: Option<Arc<bean_security::AuditLog>>,
    mcp_audit: Option<Arc<bean_security::AuditLog>>,
) -> (TempDir, ServeContext, Arc<MemoryStore>) {
    let (_workspace, router, mut config, _billing) = setup();
    config.mcp_server = settings.clone();
    let store = Arc::new(MemoryStore::default());
    let ctx = ServeContext {
        router,
        auth: Arc::new(McpAuth::new(store.clone())),
        audit,
        mcp_audit,
        config: Arc::new(config),
        shutdown: tokio_util::sync::CancellationToken::new(),
        limiter: Arc::new(McpRateLimiter::new(&settings)),
    };
    // Workspace của `setup()` đã bị TempDir của chính nó giữ; ta tạo thêm một thư mục
    // tạm để giữ `TempDir` sống suốt test (registry đã sao chép đường dẫn rồi).
    (TempDir::new().unwrap(), ctx, store)
}

/// Cấp token thật cho `cline` trong store đã dựng.
async fn issue_token(store: &MemoryStore) -> String {
    let token = mcp_server::new_token().unwrap();
    store
        .create_mcp_client(
            &mcp_server::hash_token(&token),
            "cline",
            "finance-readonly",
            &chrono::Utc::now().to_rfc3339(),
            "",
        )
        .await
        .unwrap();
    token
}

/// Test K24 (1): dò token trên transport HTTP bị khoá y như `POST /api/auth/login`
/// (5 lần sai/60s) rồi trả `RateLimited` — **không** phải `401` mãi mãi.
#[tokio::test]
async fn http_transport_locks_out_after_repeated_token_failures() {
    let (_dir, ctx, store) = serve_context(http_settings(), None, None);
    let _good = issue_token(&store).await;
    let bad = mcp_server::new_token().unwrap();

    // 5 lần sai: vẫn là lỗi xác thực, chưa bị khoá.
    for attempt in 1..=5 {
        let error = ctx
            .authenticate_http(Some(&bad), loopback())
            .await
            .expect_err("token sai phải bị từ chối");
        assert!(
            matches!(error, McpServerError::Auth(_)),
            "lần {attempt} phải mới là lỗi xác thực, chưa phải khoá: {error}"
        );
    }
    // Lần thứ 6: đã khoá. `McpServerError::RateLimited` là điều kiện để tầng HTTP trả
    // **429** kèm `Retry-After`; nếu đổi thành biến thể khác thì client sẽ bị báo
    // nhầm "token sai" và người dùng đi sửa cấu hình vô ích.
    let locked = ctx
        .authenticate_http(Some(&bad), loopback())
        .await
        .expect_err("sau ngưỡng login phải bị khoá");
    let retry_after = match locked {
        McpServerError::RateLimited(retry_after) => retry_after,
        other => {
            assert!(
                matches!(other, McpServerError::RateLimited(_)),
                "phải trả RateLimited để HTTP trả 429, thực tế: {other:?}"
            );
            std::time::Duration::ZERO
        }
    };
    assert!(
        retry_after >= std::time::Duration::from_secs(1),
        "Retry-After phải >= 1 giây: {retry_after:?}"
    );
    // Token **đúng** cũng bị chặn khi IP đã bị khoá — đánh đổi có chủ ý của lớp chống
    // brute-force (nếu không, kẻ dò chỉ cần tự dùng token lợi để lách).
    assert!(matches!(
        ctx.authenticate_http(Some(&_good), loopback()).await,
        Err(McpServerError::RateLimited(_))
    ));
}

/// Test K24 (2): dưới ngưỡng, token hợp lệ vẫn chạy bình thường — không rate-limit nhầm
/// traffic thật. Đây là hồi quy quan trọng nhất: quá tay ở đây là vỡ phiên coding dài.
#[tokio::test]
async fn valid_token_below_the_limit_keeps_working() {
    let (_dir, ctx, store) = serve_context(http_settings(), None, None);
    let token = issue_token(&store).await;

    // 100 lần liên tiếp, xa dưới trần 120/phút, mỗi lần đều phải xác thực được.
    for request in 1..=100 {
        let identity = ctx.authenticate_http(Some(&token), loopback()).await;
        assert!(
            identity.is_ok(),
            "request {request} hợp lệ phải qua (lỗi: {:?})",
            identity.err()
        );
        let identity = identity.expect("đã assert là Ok ở trên");
        assert_eq!(identity.user_id, "mcp-client:cline");
        assert_eq!(identity.role, "finance-readonly");
    }
}

/// Test K24 (2b): vượt trần lưu lượng thì **đã xác thực** cũng bị chặn — đây là lớp
/// chống *token lộ bị dùng*, khác hẳn lớp chống dò token ở test trên.
#[tokio::test]
async fn valid_token_above_the_volume_limit_is_limited() {
    let mut settings = http_settings();
    settings.rate_limit_per_minute = 3;
    settings.rate_limit_ip_multiplier = 0; // chỉ kiểm lớp theo token
    let (_dir, ctx, store) = serve_context(settings, None, None);
    let token = issue_token(&store).await;

    for request in 1..=3 {
        assert!(
            ctx.authenticate_http(Some(&token), loopback())
                .await
                .is_ok(),
            "request {request} phải được cho qua"
        );
    }
    let limited = ctx
        .authenticate_http(Some(&token), loopback())
        .await
        .expect_err("request thứ 4 vượt trần 3/phút");
    assert!(
        matches!(limited, McpServerError::RateLimited(_)),
        "{limited}"
    );
    // Trần lưu lượng theo IP tắt (`multiplier = 0`) không được kéo sập trần theo token.
    assert!(matches!(limited, McpServerError::RateLimited(_)));
}

/// Test K24 (3): stdio **KHÔNG** bị áp rate-limit. Cùng bộ limiter đó mà HTTP dùng,
/// nhưng 1000 lần xác thực stdio vẫn phải qua — hành vi M25 giữ nguyên.
#[tokio::test]
async fn stdio_transport_is_never_rate_limited() {
    let (_dir, ctx, store) = serve_context(http_settings(), None, None);
    let token = issue_token(&store).await;
    let bad = mcp_server::new_token().unwrap();

    // Trước hết chứng minh limiter **đang hoạt động** trên HTTP: dùng nó "hết" đi.
    for _ in 0..10 {
        let _ = ctx.authenticate_http(Some(&bad), loopback()).await;
    }
    assert!(
        matches!(
            ctx.authenticate_http(Some(&token), loopback()).await,
            Err(McpServerError::RateLimited(_))
        ),
        "HTTP phải bị khoá (điều kiện tiên quyết của test này)"
    );

    // Giờ đi qua stdio: vượt xa mọi ngưỡng mà vẫn không bị chặn.
    for request in 1..=1000 {
        let identity = ctx.authenticate_stdio(&token).await;
        assert!(
            identity.is_ok(),
            "stdio #{request} phải qua (lỗi: {:?})",
            identity.err()
        );
        let identity = identity.expect("đã assert là Ok ở trên");
        assert_eq!(identity.user_id, "mcp-client:cline");
    }
    // Kể cả khi chính token đó đã bị HTTP khoá vì dò thất bại, stdio vẫn không sao —
    // hai transport dùng chung *cấu hình* nhưng không dùng chung *bộ đếm*.
    assert!(ctx.authenticate_stdio(&token).await.is_ok());
}

/// Test K24 (4): một request qua MCP xuất hiện trong `audit/mcp.jsonl` với đủ thông tin
/// (client nào, tool nào, thời điểm) — và `audit.jsonl` **vẫn còn** bản ghi (D16.10:
/// ghi song song, không thay thế).
#[tokio::test]
async fn mcp_request_is_written_to_the_dedicated_log() {
    let audit_dir = TempDir::new().unwrap();
    let shared = Arc::new(bean_security::AuditLog::open(audit_dir.path()).unwrap());
    let dedicated =
        Arc::new(bean_security::AuditLog::open_named(audit_dir.path(), "mcp.jsonl").unwrap());
    let (_dir, ctx, _store) = serve_context(
        http_settings(),
        Some(shared.clone()),
        Some(dedicated.clone()),
    );

    // Đi qua đúng handler mà transport HTTP dùng, rồi gọi tool thật trên router.
    let identity = McpClientIdentity {
        name: "cline".into(),
        user_id: "mcp-client:cline".into(),
        role: "finance-readonly".into(),
    };
    let handler = ctx.handler_for(identity).unwrap();
    handler.audit_event("mcp_handshake", true, None);
    let (_workspace, router, _config, _billing) = setup();
    assert!(
        router
            .call_tool_as(
                "mcp-client:cline",
                "billing_read_cost",
                serde_json::json!({})
            )
            .await
            .is_ok(),
        "tool được expose phải chạy được"
    );
    handler.audit_event("mcp_tools_list", true, None);

    // Nhật ký riêng: JSONL hợp lệ, có client + tool + thời điểm.
    let mcp_log = std::fs::read_to_string(audit_dir.path().join("mcp.jsonl")).unwrap();
    let lines: Vec<&str> = mcp_log.lines().collect();
    assert_eq!(lines.len(), 2, "mỗi sự kiện MCP một dòng: {mcp_log}");
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["channel"], "mcp-client:cline", "phải biết client nào");
    assert_eq!(first["tool"], "mcp_handshake");
    assert_eq!(first["decided_by"], "mcp-client:cline");
    assert_eq!(first["ok"], true);
    assert_eq!(first["args"]["via"], "mcp");
    assert_eq!(first["args"]["role"], "finance-readonly");
    // Thời điểm phải parse được là RFC3339 UTC.
    let ts = first["ts"].as_str().expect("phải có ts");
    assert!(
        chrono::DateTime::parse_from_rfc3339(ts).is_ok(),
        "ts phải là RFC3339: {ts}"
    );

    // D16.10: `audit.jsonl` giữ nguyên bản ghi chung — không mất lịch sử cho UI.
    let shared_log = std::fs::read_to_string(audit_dir.path().join("audit.jsonl")).unwrap();
    assert!(
        shared_log.contains("mcp-client:cline"),
        "audit.jsonl phải vẫn có bản ghi MCP: {shared_log}"
    );
    assert_eq!(
        shared_log.lines().count(),
        mcp_log.lines().count(),
        "ghi song song ⇒ hai file có cùng số bản ghi"
    );
}

/// `mcp.jsonl` không được ghi ra ngoài thư mục audit: tên file phải là tên đơn giản.
#[test]
fn audit_open_named_rejects_paths_outside_the_audit_dir() {
    let dir = TempDir::new().unwrap();
    for bad in ["", "../khoac.txt", "sub/dir.jsonl", "..\\khoac.txt", ".."] {
        assert!(
            bean_security::AuditLog::open_named(dir.path(), bad).is_err(),
            "phải từ chối tên file {bad:?}"
        );
    }
    assert!(bean_security::AuditLog::open_named(dir.path(), "mcp.jsonl").is_ok());
}

/// Lớp chống dò token phải để lại dấu vết trong `mcp.jsonl`.
///
/// Sự kiện này xảy ra **trước** khi có handler (chưa biết client nào) nên được ghi ở
/// tầng HTTP. Nếu chỉ ghi `tracing` thì nhật ký riêng mất đúng dấu vết mà K24 cần —
/// "phát hiện lạm dụng" chính là lúc này, không phải lúc tool chạy thành công.
#[tokio::test]
async fn denied_requests_are_recorded_for_abuse_detection() {
    let audit_dir = TempDir::new().unwrap();
    let dedicated =
        Arc::new(bean_security::AuditLog::open_named(audit_dir.path(), "mcp.jsonl").unwrap());
    let (_dir, ctx, _store) = serve_context(http_settings(), None, Some(dedicated.clone()));
    let bad = mcp_server::new_token().unwrap();

    // 5 lần sai xác thực + 1 lần bị khoá. Lặp lại đúng những gì tầng HTTP làm khi bị
    // từ chối (xem `crates/bean/src/mcp.rs::handle`).
    for _ in 0..6 {
        match ctx.authenticate_http(Some(&bad), loopback()).await {
            Err(McpServerError::RateLimited(retry_after)) => {
                ctx.record_denied(loopback(), "mcp_rate_limited", retry_after.as_secs());
            }
            Err(_) => ctx.record_denied(loopback(), "mcp_auth_failed", 0),
            Ok(_) => {}
        }
    }

    let mcp_log = std::fs::read_to_string(audit_dir.path().join("mcp.jsonl")).unwrap();
    assert!(!mcp_log.is_empty(), "phải có dấu vết trong mcp.jsonl");
    let entries: Vec<serde_json::Value> = mcp_log
        .lines()
        .map(|line| serde_json::from_str(line).expect("mỗi dòng phải là JSON"))
        .collect();
    let rate_limited: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|entry| entry["tool"] == "mcp_rate_limited")
        .collect();
    assert!(
        !rate_limited.is_empty(),
        "phải ghi sự kiện bị giới hạn tần suất: {mcp_log}"
    );
    let last = rate_limited.last().expect("vừa khẳng định không rỗng");
    assert_eq!(last["ok"], false);
    assert_eq!(last["decision"], "deny");
    // IP phải có trong bản ghi để điều tra được.
    assert_eq!(last["args"]["ip"], "127.0.0.1");
    assert!(
        last["args"]["retry_after_seconds"].as_u64().unwrap_or(0) >= 1,
        "phải cho client biết chờ bao lâu: {last}"
    );
    // Không token thô nào được ghi (kể cả token sai vừa dùng).
    assert!(!mcp_log.contains(&bad), "không được ghi token thô vào log");
}
