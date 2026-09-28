//! Test round-trip serde cho `Message` (agents.md mục 5, 20).
//!
//! Bất biến được bảo vệ ở đây: cặp `assistant(tool_calls)` + `tool` result phải giữ nguyên
//! `id`/`tool_call_id` sau khi serialize → deserialize (nếu lệch, provider sẽ trả 400).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bean_types::llm::LlmResponse;
use bean_types::{Message, Role, ToolCall};
use serde_json::json;

#[test]
fn role_serializes_as_snake_case() {
    assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
    assert_eq!(
        serde_json::to_string(&Role::Assistant).unwrap(),
        "\"assistant\""
    );
    assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
}

#[test]
fn user_message_round_trips() {
    let message = Message::user("xin chào 👋");
    let json = serde_json::to_string(&message).unwrap();
    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(message, back);
    assert_eq!(back.role, Role::User);
    assert_eq!(back.text.as_deref(), Some("xin chào 👋"));
    assert!(back.tool_calls.is_empty());
    assert!(back.tool_call_id.is_none());
    assert!(!back.is_error);
}

#[test]
fn tool_pair_stays_consistent_through_serde() {
    let call = ToolCall::new("call_1", "read_file", json!({ "path": "a.txt" }));
    let assistant = Message::assistant(Some("đọc file".to_string()), vec![call.clone()]);
    let result = Message::tool("call_1", "nội dung file");

    let json = serde_json::to_string(&vec![assistant.clone(), result.clone()]).unwrap();
    let back: Vec<Message> = serde_json::from_str(&json).unwrap();

    assert_eq!(back[0], assistant);
    assert_eq!(back[1], result);
    assert_eq!(
        back[0].tool_calls[0].id,
        back[1].tool_call_id.clone().unwrap()
    );
    assert_eq!(back[0].tool_calls[0].args, json!({ "path": "a.txt" }));
}

#[test]
fn tool_error_sets_flag() {
    let error = Message::tool_error("call_2", "tool `x` không tồn tại");
    assert!(error.is_error);
    assert_eq!(error.role, Role::Tool);
    assert_eq!(error.tool_call_id.as_deref(), Some("call_2"));

    let ok = Message::tool("call_2", "ổn");
    assert!(!ok.is_error);
}

#[test]
fn optional_fields_may_be_absent_in_json() {
    // Provider/DB có thể bỏ các trường mặc định — không được làm parse thất bại.
    let raw = r#"{"role":"assistant","text":"chào"}"#;
    let message: Message = serde_json::from_str(raw).unwrap();
    assert_eq!(message.text.as_deref(), Some("chào"));
    assert!(message.tool_calls.is_empty());
    assert!(!message.is_error);
}

#[test]
fn from_response_keeps_text_and_tool_calls() {
    let response = LlmResponse {
        text: Some("tôi sẽ đọc file".to_string()),
        tool_calls: vec![ToolCall::new("c1", "read_file", json!({ "path": "a.txt" }))],
        ..LlmResponse::default()
    };
    let message = Message::from_response(&response);
    assert_eq!(message.role, Role::Assistant);
    assert_eq!(message.text.as_deref(), Some("tôi sẽ đọc file"));
    assert_eq!(message.tool_calls, response.tool_calls);
}

#[test]
fn text_for_search_contains_tool_name_and_args() {
    let message = Message::assistant(
        Some("đang tìm".to_string()),
        vec![ToolCall::new("c1", "grep", json!({ "pattern": "bảo mật" }))],
    );
    let haystack = message.text_for_search();
    assert!(haystack.contains("đang tìm"), "{haystack}");
    assert!(haystack.contains("[tool:grep]"), "{haystack}");
    assert!(haystack.contains("bảo mật"), "{haystack}");
}

#[test]
fn risk_levels_match_spec() {
    use bean_types::Risk;
    assert!(!Risk::Safe.needs_confirmation());
    assert!(Risk::Confirm.needs_confirmation());
    assert!(Risk::Dangerous.needs_confirmation());
    assert!(Risk::Confirm.allows_session_grant());
    assert!(
        !Risk::Dangerous.allows_session_grant(),
        "Dangerous không được có 'cho phép cả phiên'"
    );
    assert_eq!(
        serde_json::to_string(&Risk::Dangerous).unwrap(),
        "\"dangerous\""
    );
}
