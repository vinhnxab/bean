//! Unit test MCP: ham thuan + validate, khong can server that.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use std::time::Duration;

use bean_types::{Risk, config::McpServerConfig as ServerConfig};
use rmcp::model::CallToolResult;

use super::config::{
    effective_call_timeout, inherited_env, looks_sensitive, validate_spawn_config,
};
use super::tool::{has_readable_text, risk_for_trust, silent_error_payload};

/// Dựng `CallToolResult` từ đúng JSON server gửi trên wire.
///
/// `CallToolResult` là `#[non_exhaustive]` nên không dựng bằng struct literal được;
/// deserialize từ JSON vừa khả thi vừa sát thực tế hơn.
fn result_from_wire(json: serde_json::Value) -> CallToolResult {
    serde_json::from_value(json).expect("JSON kết quả MCP phải hợp lệ")
}

/// Hồi quy đúng lỗi gặp thật: `chrome-devtools-mcp` `--slim` trả
/// `{"content":[{"type":"text","text":""}],"isError":true}` — không có lý do.
#[test]
fn empty_text_block_reads_as_silent_failure() {
    let result = result_from_wire(serde_json::json!({
        "content": [{ "type": "text", "text": "" }],
        "isError": true
    }));
    assert!(!has_readable_text(&result.content));
}

#[test]
fn whitespace_only_text_is_also_silent() {
    let result = result_from_wire(serde_json::json!({
        "content": [{ "type": "text", "text": "   \n\t  " }],
        "isError": true
    }));
    assert!(
        !has_readable_text(&result.content),
        "chỉ khoảng trắng thì model cũng không đọc được gì"
    );
}

/// Có message thật thì phải giữ nguyên đường cũ — không được đụng vào kết quả hợp lệ.
#[test]
fn real_message_still_counts_as_readable() {
    let result = result_from_wire(serde_json::json!({
        "content": [{ "type": "text", "text": "Navigated to file:///tmp/a.html." }],
        "isError": false
    }));
    assert!(has_readable_text(&result.content));
}

/// Server trả về cả ảnh (mục 26) thì vẫn là nội dung đọc được, không phải im lặng.
#[test]
fn image_block_counts_as_readable_content() {
    let result = result_from_wire(serde_json::json!({
        "content": [
            { "type": "text", "text": "" },
            { "type": "image", "data": "aGVsbG8=", "mimeType": "image/png" }
        ],
        "isError": true
    }));
    assert!(has_readable_text(&result.content));
}

#[test]
fn no_content_at_all_is_silent() {
    let result = result_from_wire(serde_json::json!({ "isError": true }));
    assert!(!has_readable_text(&result.content));
}

/// Payload thay thế phải **có nội dung dùng được**: nêu tên server và chỉ cách sửa.
///
/// Trước khi có `silent_error_payload`, model nhận đúng `text: ""` — nên đây là
/// hồi quy cho đúng cái lỗi người dùng gặp phải.
#[test]
fn silent_error_payload_explains_the_cause_and_keeps_raw() {
    let raw = result_from_wire(serde_json::json!({
        "content": [{ "type": "text", "text": "" }],
        "isError": true
    }));
    let payload = silent_error_payload("chrome", &raw);

    let message = payload["error"].as_str().expect("phải có error");
    assert!(message.contains("chrome"), "phải nêu tên server: {message}");
    assert!(
        message.contains("TUYỆT ĐỐI"),
        "phải chỉ cách sửa đúng: {message}"
    );

    // `raw` giữ lại nguyên trạng để không mất thông tin gì của server.
    assert_eq!(payload["raw"]["content"][0]["text"], "");
    assert_eq!(payload["isError"], true);
}

#[test]
fn risk_is_confirm_unless_server_is_trusted() {
    assert_eq!(risk_for_trust(false), Risk::Confirm);
    assert_eq!(risk_for_trust(true), Risk::Safe);
}

fn fixture() -> ServerConfig {
    ServerConfig {
        name: "fixture".to_string(),
        command: "/bin/true".to_string(),
        args: Vec::new(),
        env: BTreeMap::new(),
        trust: false,
        tool_tags: Vec::new(),
        call_timeout_seconds: None,
        inherit_env: Vec::new(),
    }
}

#[test]
fn call_timeout_defaults_unless_configured() {
    let mut server = fixture();
    let default = Duration::from_secs(60);
    assert_eq!(effective_call_timeout(&server, default), default);

    server.call_timeout_seconds = Some(300);
    assert_eq!(
        effective_call_timeout(&server, default),
        Duration::from_secs(300)
    );
}

#[test]
fn zero_call_timeout_falls_back_instead_of_failing_every_call() {
    // `Config::validate` chặn 0, nhưng struct dựng tay thì không: `Duration::ZERO`
    // sẽ khiến mọi tool call treo tức thì, nên phải rơi về mặc định.
    let mut server = fixture();
    server.call_timeout_seconds = Some(0);
    let default = Duration::from_secs(60);
    assert_eq!(effective_call_timeout(&server, default), default);
}

#[test]
fn inherit_env_skips_names_shadowed_by_explicit_env() {
    // `env` tường minh phải thắng — kiểm tra ở mức hàm thuần, không cần spawn.
    let mut server = fixture();
    server
        .env
        .insert("PATH".to_string(), "tu-config".to_string());
    server.inherit_env = vec!["PATH".to_string()];
    assert!(
        inherited_env(&server).is_empty(),
        "biến có trong `env` không được kế thừa lần nữa"
    );
}

#[test]
fn inherit_env_ignores_blank_and_missing_variables() {
    let mut server = fixture();
    server.inherit_env = vec!["  ".to_string(), "BEAN_BIEN_KHONG_TON_TAI_XYZ".to_string()];
    assert!(inherited_env(&server).is_empty());
}

#[test]
fn inherit_env_returns_existing_variable_value() {
    // `PATH` gần như luôn tồn tại; đây là biến mà `env_clear()` xoá đi.
    let mut server = fixture();
    server.inherit_env = vec!["PATH".to_string()];
    let Some(expected) = std::env::var("PATH").ok() else {
        return;
    };
    let resolved = inherited_env(&server);
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(resolved[0], ("PATH".to_string(), expected));
}

#[test]
fn sensitive_names_are_flagged_for_the_operator() {
    // Cảnh báo, không chặn: truyền GITHUB_TOKEN cho MCP server GitHub là hợp lệ.
    for name in [
        "GITHUB_TOKEN",
        "OPENAI_API_KEY",
        "my_password",
        "AWS_SECRET_ACCESS_KEY",
        "DB_CREDENTIAL",
        "auth_authorization",
    ] {
        assert!(looks_sensitive(name), "{name} phải bị cảnh báo");
    }
    for name in ["PATH", "HOME", "DISPLAY", "LANG", "USER", "XDG_RUNTIME_DIR"] {
        assert!(!looks_sensitive(name), "{name} không phải secret");
    }
}

#[test]
fn invalid_inherit_env_names_are_rejected_before_spawn() {
    // Lưới an toàn cuối: chặn ở đây thay vì để `Command::env` panic.
    for bad in ["", "  ", "A=B", "HAS\0NUL"] {
        let mut server = fixture();
        server.inherit_env = vec![bad.to_string()];
        // Chỉ dùng `assert!`: workspace cấm cả `panic!` lẫn `unwrap_used` trong
        // unit test nằm trong `src/` (khác với test tích hợp trong `tests/`).
        let rejected = match validate_spawn_config(&server) {
            Err(error) => error.to_string().contains("inherit_env"),
            Ok(()) => false,
        };
        assert!(rejected, "inherit_env = {bad:?} phải bị chặn");
    }
}
