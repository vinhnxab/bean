//! Test mức rủi ro theo origin của nhóm `browser-act` (M26).
//!
//! Bằng chứng bắt buộc:
//!
//! * Origin **ngoài** whitelist luôn `Dangerous` **kể cả khi gọi liên tiếp cùng
//!   phiên** — tức `Policy` không bao giờ hiện tuỳ chọn "cho phép trong phiên".
//! * Origin **trong** whitelist là `Confirm`, và "cho phép trong phiên" **có** hiệu
//!   lực: lần gọi thứ hai chạy thẳng, không hỏi lại.
//!
//! Test chạy `bean_security::policy::Policy` thật chứ không chỉ kiểm
//! `Tool::risk`, vì quyết định "có cho phép trong phiên hay không" nằm ở `Policy`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use bean_browser::build_tools;
use bean_security::policy::{PolicyDecision, SessionPolicy, decide};
use bean_tools::ToolRegistry;
use bean_types::Risk;
use bean_types::config::BrowserConfig;

/// Trả về **cả** registry lẫn `SessionManager` — chúng phải là **cùng một** cặp,
/// vì tool `browser-click` tra origin trên đúng session đó. Tách đôi sẽ khiến
/// test đặt origin vào một manager mà tool không hề thấy.
fn registry(origins: &[&str]) -> (ToolRegistry, std::sync::Arc<bean_browser::SessionManager>) {
    let dir = tempfile::tempdir().expect("tạo thư mục tạm");
    let leaked: &'static std::path::Path = Box::leak(dir.path().to_path_buf().into_boxed_path());
    let settings = BrowserConfig {
        enabled: true,
        allowed_origins: origins.iter().map(|s| (*s).to_string()).collect(),
        ..BrowserConfig::default()
    };
    let (tools, session) = build_tools(&settings, leaked);
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool).expect("đăng ký tool browser");
    }
    (registry, session)
}

fn args(url: &str) -> serde_json::Value {
    serde_json::json!({ "url": url })
}

/// Origin trong whitelist ⇒ `Confirm` (được tuỳ chọn "cho phép trong phiên").
#[test]
fn whitelisted_origin_is_confirm_with_session_option() {
    let (registry, _session) = registry(&["http://localhost:3000", "https://*.dev.internal"]);
    let tool = registry.get("browser_navigate").expect("tool tồn tại");
    assert_eq!(tool.risk(&args("http://localhost:3000/app")), Risk::Confirm);

    let session = SessionPolicy::new();
    let decision = decide(
        "browser_navigate",
        Risk::Confirm,
        &args("http://localhost:3000/app"),
        false,
        &session,
    );
    assert_eq!(
        decision,
        PolicyDecision::NeedsConfirm {
            allow_in_session: true
        },
        "origin trong whitelist phải có tuỳ chọn \"cho phép trong phiên\""
    );

    // Lần hai sau khi người dùng bấm "cho phép trong phiên": chạy thẳng.
    session.allow("browser_navigate");
    assert_eq!(
        decide(
            "browser_navigate",
            Risk::Confirm,
            &args("http://localhost:3000/app"),
            false,
            &session
        ),
        PolicyDecision::Allowed,
        "sau khi cho phép trong phiên thì không hỏi lại"
    );
}

/// Origin NGOÀI whitelist ⇒ `Dangerous`, và gọi liên tiếp bao nhiêu lần cũng vậy.
#[test]
fn outside_origin_is_always_dangerous_even_when_repeated_in_session() {
    let (registry, _session) = registry(&["http://localhost:3000"]);
    let tool = registry.get("browser_navigate").expect("tool tồn tại");
    let outside = args("https://production.example.com/admin");
    assert_eq!(tool.risk(&outside), Risk::Dangerous);

    let session = SessionPolicy::new();
    for round in 1..=3 {
        let decision = decide(
            "browser_navigate",
            Risk::Dangerous,
            &outside,
            false,
            &session,
        );
        assert_eq!(
            decision,
            PolicyDecision::NeedsConfirm {
                allow_in_session: false
            },
            "lần {round}: origin ngoài whitelist phải luôn hỏi, KHÔNG có tuỳ chọn trong phiên"
        );
        // Kể cả khi người dùng (hoặc đường nào đó) đã ép allow, Dangerous vẫn hỏi.
        session.allow("browser_navigate");
    }
}

/// Wildcard khớp đúng ranh giới label ở **tầng risk** (không chỉ ở tầng so khớp).
#[test]
fn wildcard_origin_risk_follows_label_boundary() {
    let (registry, _session) = registry(&["https://*.dev.internal"]);
    let tool = registry.get("browser_navigate").expect("tool tồn tại");
    assert_eq!(
        tool.risk(&args("https://api.dev.internal/x")),
        Risk::Confirm,
        " subdomain thật phải khớp wildcard"
    );
    assert_eq!(
        tool.risk(&args("https://dev.internal.attacker.com/x")),
        Risk::Dangerous,
        "domain giả dạng hậu tố phải ngoài whitelist"
    );
}

/// Whitelist rỗng ⇒ mọi origin `Dangerous` (kể cả localhost).
#[test]
fn empty_whitelist_makes_every_origin_dangerous() {
    let (registry, _session) = registry(&[]);
    let tool = registry.get("browser_navigate").expect("tool tồn tại");
    for url in [
        "http://localhost:3000",
        "https://example.com",
        "https://api.dev.internal",
    ] {
        assert_eq!(tool.risk(&args(url)), Risk::Dangerous, "{url}");
    }
}

/// URL lỗi/không xác định được origin ⇒ `Dangerous` (fail-closed), không phải Confirm.
#[test]
fn unparseable_or_blocked_url_fails_closed_to_dangerous() {
    let (registry, _session) = registry(&["http://localhost:3000"]);
    let tool = registry.get("browser_navigate").expect("tool tồn tại");
    for bad in [
        "",
        "không phải url",
        "javascript:alert(1)",
        // Ngoài whitelist nhưng trỏ vào mạng nội bộ: guard chặn, risk phải Dangerous.
        "http://169.254.169.254/latest/meta-data/",
    ] {
        assert_eq!(tool.risk(&args(bad)), Risk::Dangerous, "{bad:?}");
    }
    // Thiếu hẳn `url` cũng phải Dangerous, không được mặc định Confirm.
    assert_eq!(tool.risk(&serde_json::json!({})), Risk::Dangerous);
}

/// Tool không mang URL dùng origin hiện tại; chưa navigate ⇒ `Dangerous`.
#[tokio::test]
async fn tools_without_url_are_dangerous_until_a_navigation_sets_the_origin() {
    let (registry, session_manager) = registry(&["http://localhost:3000"]);

    for name in ["browser_click", "browser_fill", "browser_press_key"] {
        let tool = registry.get(name).expect("tool tồn tại");
        // Chưa có navigate ⇒ không xác định được origin ⇒ Dangerous.
        assert_eq!(
            tool.risk(&serde_json::json!({ "selector": "#x" })),
            Risk::Dangerous,
            "{name} phải Dangerous khi chưa navigate"
        );
    }

    // Sau khi nhớ origin trong whitelist ⇒ Confirm.
    session_manager
        .set_current_origin(Some(
            bean_browser::parse_origin(
                &url::Url::parse("http://localhost:3000").expect("url hợp lệ"),
            )
            .expect("origin hợp lệ"),
        ))
        .await;

    for name in ["browser_click", "browser_fill", "browser_press_key"] {
        let tool = registry.get(name).expect("tool tồn tại");
        assert_eq!(
            tool.risk(&serde_json::json!({ "selector": "#x" })),
            Risk::Confirm,
            "{name} phải Confirm khi origin đã nằm trong whitelist"
        );
    }
}

/// Bảo đảm hai registry dùng chung hằng số tag — chống hồi quy âm thầm.
#[test]
fn registry_contains_exactly_the_nine_browser_tools() {
    let (registry, _session) = registry(&[]);
    let mut names: Vec<String> = registry
        .names()
        .into_iter()
        .filter(|n| n.starts_with("browser_"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "browser_click",
            "browser_console_logs",
            "browser_evaluate_script",
            "browser_fill",
            "browser_navigate",
            "browser_network",
            "browser_performance_trace",
            "browser_press_key",
            "browser_screenshot",
        ]
    );
}
