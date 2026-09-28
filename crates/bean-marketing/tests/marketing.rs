//! M24 — Marketing agent: tách domain, publish luôn hỏi, draft không gọi mạng.
//!
//! Bằng chứng cho ba yêu cầu kiểm thử bắt buộc của `Plan.md`:
//! 1. `marketing_role_sees_only_marketing_tools` — role `marketing` **không thấy/gọi
//!    được** tool ngoài tag `marketing-*`, kể cả `write_file`/`run_shell`.
//! 2. `publish_always_needs_confirm_even_after_session_allow` — gọi `marketing_publish`
//!    **luôn** yêu cầu Confirm, kể cả khi đã cho phép trong phiên ở lần trước.
//! 3. `draft_writes_file_without_any_network_call` — `marketing_draft` chỉ ghi file.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bean_marketing::{PublishClient, marketing_draft, marketing_publish};
use bean_security::{CapWorkspace, Policy, PolicyDecision, SessionPolicy};
use bean_tools::{ToolCtx, ToolRegistry};
use bean_types::config::{MarketingConfig, RoleConfig};
use bean_types::{Config, Risk, RolePermissions, SessionId};
use tokio_util::sync::CancellationToken;

const MARKETING_TAGS: [&str; 3] = ["marketing-read", "marketing-draft", "marketing-publish"];

fn ctx(workspace: &std::path::Path) -> ToolCtx {
    ToolCtx::for_project(
        Arc::new(CapWorkspace::open(workspace.to_path_buf()).unwrap()),
        SessionId::new(1),
        CancellationToken::new(),
        Arc::new(AtomicBool::new(false)),
    )
}

/// Quyền của vai trò marketing như khai trong cấu hình mẫu.
fn marketing_perms() -> RolePermissions {
    let set = || BTreeSet::from(MARKETING_TAGS.map(String::from));
    RolePermissions::restricted_to("marketing", set(), set())
}

fn marketing_role_config() -> RoleConfig {
    RoleConfig {
        name: "marketing".into(),
        tool_tags: MARKETING_TAGS.iter().map(|t| (*t).to_string()).collect(),
        allowed_tool_tags: MARKETING_TAGS.iter().map(|t| (*t).to_string()).collect(),
        forbid_tags: vec![],
        context_budget_tokens: None,
        daily_token_budget: None,
    }
}

/// Registry chứa tool của **nhiều domain**, để chứng minh marketing bị chặn đúng chỗ.
fn domain_registry(workspace: &std::path::Path) -> ToolRegistry {
    let ws = Arc::new(CapWorkspace::open(workspace.to_path_buf()).unwrap());
    let mut reg = ToolRegistry::with_workspace(ws);
    for tool in bean_tools::builtin::file_tools() {
        reg.register(tool).unwrap();
    }
    reg.register(marketing_draft()).unwrap();
    reg.register(marketing_publish(PublishClient::new(
        &MarketingConfig::default(),
        None,
    )))
    .unwrap();
    reg
}

/// (1) Role `marketing` chỉ thấy tool `marketing-*` — không thấy tool của domain khác.
#[test]
fn marketing_role_sees_only_marketing_tools() {
    let dir = tempfile::tempdir().unwrap();
    let reg = domain_registry(dir.path());
    let seen: Vec<String> = reg
        .specs_visible_to(&marketing_perms())
        .into_iter()
        .map(|spec| spec.name)
        .collect();

    for expected in ["marketing_draft", "marketing_publish"] {
        assert!(
            seen.contains(&expected.to_string()),
            "phải thấy {expected}: {seen:?}"
        );
    }
    // Tool của domain khác phải bị ẩn — kể cả tool an toàn.
    for hidden in ["read_file", "write_file", "list_dir"] {
        assert!(
            !seen.contains(&hidden.to_string()),
            "marketing KHÔNG được thấy tool `{hidden}`: {seen:?}"
        );
    }
}

/// (1b) Chốt chặn thứ hai ở tầng thực thi: model bịa tên tool cũng không chạy được.
#[test]
fn marketing_cannot_call_other_domains_even_by_inventing_the_name() {
    let dir = tempfile::tempdir().unwrap();
    let reg = domain_registry(dir.path());
    let perms = marketing_perms();
    for blocked in [
        "read_file",
        "write_file",
        "run_shell",
        "security_scan",
        "billing_read_cost",
    ] {
        assert!(
            !reg.allows(blocked, &perms),
            "marketing không được gọi `{blocked}`"
        );
    }
    assert!(reg.allows("marketing_draft", &perms));
}

/// Role cũ **không bị ảnh hưởng** khi thêm cơ chế danh sách trắng (rỗng = hành vi M21).
#[test]
fn roles_without_allowlist_keep_seeing_untagged_tools() {
    let dir = tempfile::tempdir().unwrap();
    let reg = domain_registry(dir.path());
    let perms = RolePermissions::restricted_to(
        "developer",
        BTreeSet::from(["dev-write".to_string()]),
        BTreeSet::new(),
    );
    let seen: Vec<String> = reg
        .specs_visible_to(&perms)
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert!(
        seen.contains(&"read_file".to_string()),
        "role không có danh sách trắng vẫn thấy tool untagged như cũ: {seen:?}"
    );
}

/// (2) `marketing_publish` LUÔN Dangerous ⇒ không có "cho phép trong phiên" (mục 7.2).
#[test]
fn publish_always_needs_confirm_even_after_session_allow() {
    let tool = marketing_publish(PublishClient::new(&MarketingConfig::default(), None));
    let args = serde_json::json!({ "text": "bài đăng" });
    assert_eq!(tool.risk(&args), Risk::Dangerous);

    let policy = Policy::new();
    let session = SessionPolicy::default();
    assert_eq!(
        policy.decide(
            "marketing_publish",
            tool.risk(&args),
            &args,
            false,
            &session
        ),
        PolicyDecision::NeedsConfirm {
            allow_in_session: false
        }
    );
    // Người dùng đã "cho phép trong phiên" ⇒ vẫn phải hỏi.
    session.allow("marketing_publish");
    assert_eq!(
        policy.decide(
            "marketing_publish",
            tool.risk(&args),
            &args,
            false,
            &session
        ),
        PolicyDecision::NeedsConfirm {
            allow_in_session: false
        },
        "M24: publish KHÔNG BAO GIỜ được cho phép trong phiên"
    );
}

/// (3) `marketing_draft` chỉ ghi file, không có đường gọi mạng nào.
#[tokio::test]
async fn draft_writes_file_without_any_network_call() {
    let dir = tempfile::tempdir().unwrap();
    let tool = marketing_draft();
    let out = tool
        .call(
            &ctx(dir.path()),
            serde_json::json!({ "title": "launch-q3", "body": "Nội dung bản nháp" }),
        )
        .await
        .expect("lưu bản nháp phải chạy được");
    assert!(out.contains("launch-q3"), "{out}");

    let saved = dir.path().join("drafts/launch-q3.md");
    assert!(saved.exists(), "phải ghi file vào drafts/");
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap(),
        "Nội dung bản nháp"
    );
    // Rủi ro `Confirm`, không phải `Dangerous`: chỉ ghi file trong workspace đã jail.
    assert_eq!(
        tool.risk(&serde_json::json!({ "title": "x", "body": "y" })),
        Risk::Confirm
    );
}

/// Không ghi đè âm thầm — phải có `overwrite = true`.
#[tokio::test]
async fn draft_refuses_to_overwrite_silently() {
    let dir = tempfile::tempdir().unwrap();
    let tool = marketing_draft();
    let args = serde_json::json!({ "title": "bai", "body": "v1" });
    tool.call(&ctx(dir.path()), args.clone()).await.unwrap();

    let err = tool
        .call(&ctx(dir.path()), args.clone())
        .await
        .expect_err("phải từ chối ghi đè khi chưa cho phép");
    assert!(err.to_string().contains("overwrite"), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("drafts/bai.md")).unwrap(),
        "v1",
        "nội dung cũ phải được giữ nguyên"
    );
}

/// `title` là đường dẫn ⇒ từ chối ở tầng tham số (không đợi tới file jail).
#[tokio::test]
async fn draft_rejects_path_traversal_in_title() {
    let dir = tempfile::tempdir().unwrap();
    let tool = marketing_draft();
    for bad in ["../escape", "a/b", "..", ""] {
        let err = tool
            .call(
                &ctx(dir.path()),
                serde_json::json!({ "title": bad, "body": "x" }),
            )
            .await
            .expect_err("title chứa đường dẫn phải bị từ chối");
        assert!(err.to_string().contains("title"), "{bad}: {err}");
    }
}

/// Chưa cấu hình ⇒ **chế độ stub**: nói rõ chưa đăng gì, không giả vờ thành công.
#[tokio::test]
async fn publish_stub_does_not_pretend_to_have_posted() {
    let dir = tempfile::tempdir().unwrap();
    let tool = marketing_publish(PublishClient::new(&MarketingConfig::default(), None));
    let out = tool
        .call(&ctx(dir.path()), serde_json::json!({ "text": "bài" }))
        .await
        .expect("stub phải trả về thông báo, không lỗi");
    assert!(out.contains("CHƯA CẤU HÌNH"), "{out}");
    assert!(out.contains("chưa có gì được đăng"), "{out}");
}

/// Cấu hình đủ nhưng endpoint không tồn tại ⇒ lỗi phải bọc untrusted (đã chạm nguồn ngoài lõi).
#[tokio::test]
async fn publish_error_is_wrapped_as_untrusted() {
    let dir = tempfile::tempdir().unwrap();
    let config = MarketingConfig {
        enabled: true,
        api_key_env: "MKT_TEST_KEY".into(),
        // `.invalid` không bao giờ phân giải được ⇒ chắc chắn lỗi, không gửi tới host thật.
        base_url: Some("https://marketing-test.invalid/v2/posts".into()),
        text_field: "text".into(),
    };
    let tool = marketing_publish(PublishClient::new(
        &config,
        Some(secrecy::SecretString::from("k")),
    ));
    let seen = Arc::new(AtomicBool::new(false));
    let c = ToolCtx::for_project(
        Arc::new(CapWorkspace::open(dir.path().to_path_buf()).unwrap()),
        SessionId::new(1),
        CancellationToken::new(),
        seen.clone(),
    );
    let err = tool
        .call(&c, serde_json::json!({ "text": "bài" }))
        .await
        .expect_err("endpoint không tồn tại thì phải lỗi");
    assert!(
        err.to_string().contains("<untrusted_content>"),
        "lỗi từ nguồn ngoài lõi phải bọc untrusted: {err}"
    );
    assert!(
        seen.load(Ordering::SeqCst),
        "phải bật cờ untrusted kể cả ở nhánh lỗi"
    );
}

/// `validate_marketing`: credential phải RIÊNG (D15.3).
#[test]
fn marketing_credential_must_not_be_shared() {
    let mut config = Config::default();
    config.marketing.api_key_env = "ANTHROPIC_API_KEY".into();
    let err = config
        .validate()
        .expect_err("trùng với key LLM phải bị từ chối");
    assert!(err.to_string().contains("riêng"), "{err}");

    config.marketing.api_key_env = "TELEGRAM_BOT_TOKEN".into();
    let err = config
        .validate()
        .expect_err("trùng với token Telegram phải bị từ chối");
    assert!(err.to_string().contains("riêng"), "{err}");

    config.marketing.api_key_env = "MARKETING_POST_ONLY_KEY".into();
    config.validate().expect("biến riêng thì hợp lệ");
}

/// Danh sách trắng không có tag nào được cấp ⇒ cấu hình im lặng, phải chặn lúc load.
#[test]
fn allowlist_without_matching_granted_tag_is_rejected() {
    let mut config = Config {
        roles: vec![RoleConfig {
            tool_tags: vec!["dev-write".into()],
            ..marketing_role_config()
        }],
        ..Config::default()
    };
    let err = config
        .validate()
        .expect_err("phải chặn role không thấy tool nào");
    assert!(err.to_string().contains("không thấy tool nào"), "{err}");
}

/// Tag `*` trong danh sách trắng là mâu thuẫn — phải chặn.
#[test]
fn wildcard_in_allowlist_is_rejected() {
    let mut config = Config {
        roles: vec![RoleConfig {
            tool_tags: vec!["*".into()],
            allowed_tool_tags: vec!["*".into()],
            ..marketing_role_config()
        }],
        ..Config::default()
    };
    let err = config
        .validate()
        .expect_err("`*` trong allowlist phải bị từ chối");
    assert!(err.to_string().contains("allowed_tool_tags"), "{err}");
}

/// Cấu hình mẫu phải parse và validate được — bảo đảm ví dụ trong tài liệu luôn chạy được.
#[test]
fn example_marketing_role_validates() {
    let mut user_roles = std::collections::BTreeMap::new();
    user_roles.insert("web:content".to_string(), "marketing".to_string());
    let mut config = Config {
        agent: bean_types::config::AgentConfig {
            user_roles,
            ..Default::default()
        },
        roles: vec![marketing_role_config()],
        ..Config::default()
    };
    config
        .validate()
        .expect("role marketing trong cấu hình mẫu phải hợp lệ");

    let perms = config.permissions_for("web:content");
    assert_eq!(perms.role, "marketing");
    assert!(perms.allowed_tool_tags.contains("marketing-publish"));
}
