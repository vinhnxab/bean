//! Test cho `FakeProvider` (agents.md mục 20: "mọi test vòng lặp dùng FakeProvider").
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use bean_llm::{ChatRequest, FakeProvider, LlmError, LlmProvider};
use bean_types::llm::{LlmResponse, StopReason};
use bean_types::{Message, ToolCall};
use serde_json::json;

fn request<'a>(messages: &'a [Message]) -> ChatRequest<'a> {
    ChatRequest {
        system: "test",
        messages,
        tools: &[],
        max_tokens: 128,
    }
}

#[tokio::test]
async fn responses_are_returned_in_order() {
    let provider = FakeProvider::new(vec![
        LlmResponse::text_only("một"),
        LlmResponse::with_tool_calls(vec![ToolCall::new(
            "c1",
            "read_file",
            json!({ "path": "a" }),
        )]),
        LlmResponse::text_only("ba"),
    ]);

    let messages = [Message::user("bắt đầu")];
    let first = provider.chat(request(&messages)).await.unwrap();
    assert_eq!(first.text.as_deref(), Some("một"));
    assert_eq!(first.stop, StopReason::EndTurn);

    let second = provider.chat(request(&messages)).await.unwrap();
    assert_eq!(second.stop, StopReason::ToolUse);
    assert_eq!(second.tool_calls.len(), 1);
    assert_eq!(second.tool_calls[0].name, "read_file");

    let third = provider.chat(request(&messages)).await.unwrap();
    assert_eq!(third.text.as_deref(), Some("ba"));
}

#[tokio::test]
async fn exhausted_script_returns_error_not_panic() {
    let provider = FakeProvider::new(vec![LlmResponse::text_only("chỉ một")]);
    let messages = [Message::user("x")];
    assert!(provider.chat(request(&messages)).await.is_ok());

    let err = provider.chat(request(&messages)).await.unwrap_err();
    match err {
        LlmError::FakeScriptExhausted {
            requested,
            available,
        } => {
            assert_eq!(requested, 2);
            assert_eq!(available, 1);
        }
        other => panic!("phải là FakeScriptExhausted, nhận {other:?}"),
    }
}

#[tokio::test]
async fn echo_mode_returns_last_user_text() {
    let provider = FakeProvider::echo();
    assert!(provider.is_echo());

    let messages = [
        Message::user("lượt 1"),
        Message::assistant(Some("trả lời".to_string()), vec![]),
        Message::user("lượt 2"),
    ];
    let response = provider.chat(request(&messages)).await.unwrap();
    assert_eq!(
        response.text.as_deref(),
        Some("echo: lượt 2"),
        "phải echo tin nhắn user CUỐI"
    );
}

#[test]
fn script_loads_from_json_string_in_two_shapes() {
    let with_wrapper = r#"{"responses":[{"text":"a"},{"text":"b","stop":"max_tokens"}]}"#;
    let provider = FakeProvider::from_json_str(with_wrapper).unwrap();
    assert_eq!(provider.remaining(), 2);

    let bare_array = r#"[{"text":"a"}]"#;
    let provider = FakeProvider::from_json_str(bare_array).unwrap();
    assert_eq!(provider.remaining(), 1);

    // Thiếu trường nào cũng có default: tool_calls rỗng, usage 0, stop = other.
    let minimal = FakeProvider::from_json_str(r#"{"responses":[{}]}"#).unwrap();
    assert_eq!(minimal.remaining(), 1);
}

#[test]
fn broken_script_reports_clear_error() {
    let err = FakeProvider::from_json_str("{ không phải json").unwrap_err();
    assert!(matches!(err, LlmError::FakeScript(_)), "nhận {err:?}");

    // Khoá lạ trong kịch bản cũng bị từ chối (deny_unknown_fields).
    let err = FakeProvider::from_json_str(r#"{"responses":[],"typo":1}"#).unwrap_err();
    assert!(matches!(err, LlmError::FakeScript(_)), "nhận {err:?}");
}

#[test]
fn script_loads_from_file_and_missing_file_is_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("kichban.json");
    std::fs::write(&path, r#"{"responses":[{"text":"từ file"}]}"#).unwrap();
    let provider = FakeProvider::from_json_path(&path).unwrap();
    assert_eq!(provider.remaining(), 1);

    let missing = FakeProvider::from_json_path(&dir.path().join("khong-ton-tai.json")).unwrap_err();
    assert!(
        matches!(missing, LlmError::FakeScript(_)),
        "nhận {missing:?}"
    );
}

#[test]
fn is_object_safe_for_arc_dyn() {
    // Các kênh/agent loop giữ provider dưới dạng `Arc<dyn LlmProvider>`.
    let provider: Arc<dyn LlmProvider> = Arc::new(FakeProvider::echo());
    assert_eq!(provider.name(), "fake");
}

#[test]
fn retry_classification_follows_spec() {
    assert!(LlmError::Transport("mất mạng".to_string()).is_retryable());
    assert!(LlmError::Timeout.is_retryable());
    assert!(LlmError::RateLimited { retry_after: None }.is_retryable());
    assert!(
        LlmError::HttpStatus {
            status: 500,
            body: String::new()
        }
        .is_retryable()
    );
    assert!(
        LlmError::HttpStatus {
            status: 503,
            body: String::new()
        }
        .is_retryable()
    );

    // 4xx khác thì KHÔNG retry (agents.md mục 5).
    assert!(
        !LlmError::HttpStatus {
            status: 400,
            body: "sai schema".to_string()
        }
        .is_retryable()
    );
    assert!(
        !LlmError::HttpStatus {
            status: 401,
            body: String::new()
        }
        .is_retryable()
    );
    assert!(
        !LlmError::HttpStatus {
            status: 404,
            body: String::new()
        }
        .is_retryable()
    );
    assert!(!LlmError::Decode("json hỏng".to_string()).is_retryable());

    assert!(
        LlmError::RateLimited {
            retry_after: Some(std::time::Duration::from_secs(7))
        }
        .retry_after()
        .is_some_and(|d| d.as_secs() == 7)
    );
}
