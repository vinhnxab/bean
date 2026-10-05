//! Test RBAC của nhóm tool browser (M26).
//!
//! Bốn bằng chứng, đúng theo yêu cầu:
//!
//! 1. Role **developer** bị chặn gọi **mọi** tool `browser-act`, vẫn gọi được
//!    `browser-read`.
//! 2. Role **qa** thấy và gọi được cả hai nhóm (four-eyes chỉ chặn Developer, không
//!    chặn QA).
//! 3. Role không có tag nào (`no-access`) thấy **không** tool nào.
//! 4. `browser_evaluate_script` **luôn** `Dangerous`, kể cả origin trong whitelist.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::sync::Arc;

use bean_browser::{ACT_TAGS, READ_TAGS, build_tools};
use bean_tools::ToolRegistry;
use bean_types::config::{BrowserConfig, DEV_READ_TAG, DEV_WRITE_TAG, TEST_RUN_TAG};
use bean_types::{NO_ACCESS_ROLE, RolePermissions};

fn settings(origins: &[&str]) -> BrowserConfig {
    BrowserConfig {
        enabled: true,
        allowed_origins: origins.iter().map(|s| (*s).to_string()).collect(),
        ..BrowserConfig::default()
    }
}

fn registry(origins: &[&str]) -> (ToolRegistry, Arc<bean_browser::SessionManager>) {
    let dir = tempfile::tempdir().expect("tạo thư mục tạm");
    // Giữ thư mục tạm sống suốt đời test bằng cách rơi vào biến chưa dùng.
    let leaked: &'static std::path::Path = Box::leak(dir.path().to_path_buf().into_boxed_path());
    let (tools, session) = build_tools(&settings(origins), leaked);
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool).expect("đăng ký tool browser");
    }
    (registry, session)
}

fn role(name: &str, tags: &[&str]) -> RolePermissions {
    RolePermissions::from_tags(
        name,
        tags.iter()
            .map(|t| (*t).to_string())
            .collect::<BTreeSet<String>>(),
    )
}

const ACT_TOOLS: [&str; 5] = [
    "browser_navigate",
    "browser_click",
    "browser_fill",
    "browser_press_key",
    "browser_evaluate_script",
];
const READ_TOOLS: [&str; 4] = [
    "browser_screenshot",
    "browser_console_logs",
    "browser_network",
    "browser_performance_trace",
];

/// 1. Developer bị chặn toàn bộ `browser-act`, vẫn thấy `browser-read`.
#[test]
fn developer_cannot_call_any_browser_act_tool_but_can_read() {
    let (registry, _session) = registry(&["http://localhost:3000"]);
    let developer = role("developer", &[DEV_READ_TAG, DEV_WRITE_TAG]);

    for name in ACT_TOOLS {
        assert!(
            !registry.allows(name, &developer),
            "developer KHÔNG được gọi `{name}` — vi phạm four-eyes"
        );
        // Tầng payload: model cũng không được *thấy* tool.
        let visible: Vec<String> = registry
            .specs_visible_to(&developer)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        assert!(
            !visible.iter().any(|n| n == name),
            "developer không được thấy `{name}` trong payload, thấy: {visible:?}"
        );
    }
    for name in READ_TOOLS {
        assert!(
            registry.allows(name, &developer),
            "developer phải đọc được `{name}`"
        );
    }
}

/// 2. QA thấy và gọi được cả hai nhóm.
#[test]
fn qa_can_call_both_browser_groups() {
    let (registry, _session) = registry(&["http://localhost:3000"]);
    let qa = role("qa", &[DEV_READ_TAG, TEST_RUN_TAG]);
    for name in ACT_TOOLS.iter().chain(READ_TOOLS.iter()) {
        assert!(registry.allows(name, &qa), "qa phải gọi được `{name}`");
    }
}

/// 3. `no-access` là deny-all: không thấy tool nào, kể cả untagged.
#[test]
fn no_access_role_sees_no_browser_tool() {
    let (registry, _session) = registry(&["http://localhost:3000"]);
    let blocked = RolePermissions::deny_all(NO_ACCESS_ROLE);
    assert!(registry.specs_visible_to(&blocked).is_empty());
    for name in ACT_TOOLS.iter().chain(READ_TOOLS.iter()) {
        assert!(!registry.allows(name, &blocked), "{name}");
    }
}

/// Tag khai trên tool phải khớp đúng hằng số RBAC — chống hồi quy khi ai đó
/// đổi hằng số ở nơi khác.
#[test]
fn tool_tags_match_rbac_constants() {
    let (registry, _session) = registry(&[]);
    for name in READ_TOOLS {
        let tool = registry.get(name).expect("tool tồn tại");
        assert_eq!(&*tool.access().required_tags, &READ_TAGS[..], "{name}");
        assert!(tool.marks_untrusted(), "{name} phải khai marks_untrusted");
    }
    for name in ACT_TOOLS {
        let tool = registry.get(name).expect("tool tồn tại");
        assert_eq!(&*tool.access().required_tags, &ACT_TAGS[..], "{name}");
        assert!(tool.marks_untrusted(), "{name} phải khai marks_untrusted");
    }
    assert_eq!(ACT_TAGS, [TEST_RUN_TAG], "browser-act CHỈ dành cho QA");
    assert!(READ_TAGS.contains(&DEV_READ_TAG));
    assert!(READ_TAGS.contains(&TEST_RUN_TAG));
}

/// 4. `browser_evaluate_script` luôn `Dangerous`, kể cả origin trong whitelist.
#[test]
fn evaluate_script_is_always_dangerous_even_for_whitelisted_origin() {
    let (registry, _session) = registry(&["http://localhost:3000"]);
    let tool = registry
        .get("browser_evaluate_script")
        .expect("tool tồn tại");
    let args = serde_json::json!({ "expression": "return document.title" });
    assert_eq!(
        tool.risk(&args),
        bean_types::Risk::Dangerous,
        "chạy JS tuỳ ý phải luôn là Dangerous, không có ngoại lệ whitelist"
    );
}
