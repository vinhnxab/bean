//! Kiểm tra provider Anthropic (agents.md mục 20 — M2).
//!
//! Hai lớp: (1) test trực tiếp hàm wire (`build_anthropic_body`,
//! `parse_anthropic_response`) không cần mạng; (2) test tích hợp qua `LlmProvider::chat`
//! với `wiremock`, khẳng định header/path/shape và chính sách 429→`RateLimited`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use beanagent_llm::{
    ChatRequest, LlmError, LlmProvider, build_anthropic_body, parse_anthropic_response,
};
use beanagent_types::llm::StopReason;
use beanagent_types::{LlmDelta, Message, ToolCall, ToolSpec};
use futures_util::StreamExt;

const API_KEY: &str = "k-anthropic-test";

fn provider_for(server: &MockServer) -> beanagent_llm::AnthropicProvider {
    beanagent_llm::AnthropicProvider::new(
        "claude-test",
        SecretString::from(API_KEY),
        Some(&format!("{}/v1", server.uri())),
    )
    .unwrap()
}

fn request<'a>(messages: &'a [Message], tools: &'a [ToolSpec]) -> ChatRequest<'a> {
    ChatRequest {
        system: "bạn là trợ lý",
        messages,
        tools,
        max_tokens: 256,
    }
}

// ===== (1a) Dựng body request =====

#[test]
fn body_system_is_separate_field() {
    let messages = [Message::user("xin chào")];
    let body = build_anthropic_body(&request(&messages, &[]), "claude-test").unwrap();
    assert_eq!(body["model"], "claude-test");
    assert_eq!(body["max_tokens"], 256);
    assert_eq!(body["system"], "bạn là trợ lý");
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"][0]["type"], "text");
    assert_eq!(messages[0]["content"][0]["text"], "xin chào");
    // Không có tool → không có trường tools.
    assert!(body.get("tools").is_none());
}

#[test]
fn body_assistant_with_tool_calls_then_grouped_tool_results() {
    let assistant = Message::from_response(&beanagent_types::LlmResponse::with_tool_calls(vec![
        ToolCall::new("t1", "read_file", json!({"path": "a"})),
        ToolCall::new("t2", "list_dir", json!({})),
    ]));
    let messages = [
        Message::user("hỏi"),
        assistant,
        Message::tool("t1", "nội dung a"),
        Message::tool_error("t2", "không tìm thấy"),
    ];
    let body = build_anthropic_body(&request(&messages, &[]), "m").unwrap();
    let wire = body["messages"].as_array().unwrap();
    assert_eq!(
        wire.len(),
        3,
        "2 tool_result liên tiếp phải ghép thành 1 message"
    );
    // assistant: text rỗng (from_response không có text) + 2 block tool_use.
    assert_eq!(wire[1]["role"], "assistant");
    let blocks = wire[1]["content"].as_array().unwrap();
    let tool_use: Vec<_> = blocks.iter().filter(|b| b["type"] == "tool_use").collect();
    assert_eq!(tool_use.len(), 2);
    assert_eq!(tool_use[0]["id"], "t1");
    assert_eq!(tool_use[0]["input"]["path"], "a");
    // message user cuối: 2 block tool_result với is_error đúng.
    assert_eq!(wire[2]["role"], "user");
    let results = wire[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "t1");
    assert_eq!(results[0]["is_error"], false);
    assert_eq!(results[1]["tool_use_id"], "t2");
    assert_eq!(results[1]["is_error"], true);
}

#[test]
fn body_tools_use_input_schema_and_sanitize() {
    let tools = [ToolSpec::new(
        "read_file",
        "Đọc file",
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": { "P": { "type": "string" } },
            "type": "object",
            "properties": { "path": { "$ref": "#/$defs/P" } }
        }),
    )];
    let messages = [Message::user("x")];
    let body = build_anthropic_body(&request(&messages, &tools), "m").unwrap();
    let tool = &body["tools"][0];
    assert_eq!(tool["name"], "read_file");
    assert_eq!(tool["description"], "Đọc file");
    let schema = &tool["input_schema"];
    assert_eq!(schema["type"], "object");
    assert!(schema.get("$schema").is_none(), "$schema phải bị xoá");
    assert!(schema.get("$defs").is_none(), "$defs phải bị inline");
    assert_eq!(schema["properties"]["path"]["type"], "string");
}

#[test]
fn body_broken_history_is_config_error() {
    // Role::Tool thiếu tool_call_id.
    let broken = [
        Message::assistant(Some("gọi".into()), vec![]),
        Message::user("x"),
    ];
    let ok = build_anthropic_body(&request(&broken, &[]), "m");
    assert!(ok.is_ok());

    let empty: [Message; 0] = [];
    let err = build_anthropic_body(&request(&empty, &[]), "m").unwrap_err();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
}

// ===== (1b) Parse phản hồi =====

#[test]
fn parse_text_and_tool_use() {
    let resp = parse_anthropic_response(
        r#"{
            "content": [
                { "type": "text", "text": "Để tôi đọc file." },
                { "type": "tool_use", "id": "tu_1", "name": "read_file", "input": {"path": "a.txt"} }
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 11, "output_tokens": 7 }
        }"#,
    )
    .unwrap();
    assert_eq!(resp.text.as_deref(), Some("Để tôi đọc file."));
    assert_eq!(resp.stop, StopReason::ToolUse);
    assert_eq!(resp.usage.input_tokens, 11);
    assert_eq!(resp.usage.output_tokens, 7);
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "tu_1");
    assert_eq!(resp.tool_calls[0].name, "read_file");
    assert_eq!(resp.tool_calls[0].args["path"], "a.txt");
}

#[test]
fn parse_ignores_unknown_blocks_and_input_null() {
    let resp = parse_anthropic_response(
        r#"{
            "content": [
                { "type": "thinking", "thinking": "…" },
                { "type": "tool_use", "id": "tu_2", "name": "list_dir", "input": null }
            ],
            "stop_reason": "pause_turn",
            "usage": {}
        }"#,
    )
    .unwrap();
    assert!(resp.text.is_none());
    assert_eq!(resp.stop, StopReason::Other);
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(
        resp.tool_calls[0].args,
        json!({}),
        "input null phải thành object rỗng"
    );
}

#[test]
fn parse_stop_reason_mapping() {
    let parse = |stop: &str| {
        parse_anthropic_response(&format!(
            r#"{{ "content": [], "stop_reason": "{stop}", "usage": {{}} }}"#
        ))
        .unwrap()
        .stop
    };
    assert_eq!(parse("end_turn"), StopReason::EndTurn);
    assert_eq!(parse("tool_use"), StopReason::ToolUse);
    assert_eq!(parse("max_tokens"), StopReason::MaxTokens);
    assert_eq!(parse("stop_sequence"), StopReason::Other);
    assert_eq!(parse("refusal"), StopReason::Other);
}

#[test]
fn parse_malformed_is_decode_error() {
    let err = parse_anthropic_response("không phải json").unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "{err:?}");
    let err = parse_anthropic_response(r#"{ "content": "không phải mảng" }"#).unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "{err:?}");
}

// ===== (2) Tích hợp qua wiremock =====

#[tokio::test]
async fn chat_sends_expected_request_and_parses_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", API_KEY))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(
            json!({ "model": "claude-test", "max_tokens": 256 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{ "type": "text", "text": "xin chào lại" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 3, "output_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("xin chào")];
    let resp = provider.chat(request(&messages, &[])).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("xin chào lại"));
    assert_eq!(resp.stop, StopReason::EndTurn);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn chat_stream_parses_sse_text_usage_and_finish() {
    let server = MockServer::start().await;
    let sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Xin\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" chào 🦀\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_partial_json(json!({"stream": true})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("xin chào")];
    let mut stream = provider.chat_stream(request(&messages, &[])).await.unwrap();
    let mut deltas = Vec::new();
    while let Some(item) = stream.next().await {
        deltas.push(item.unwrap());
    }
    let text = deltas
        .iter()
        .filter_map(|delta| match delta {
            LlmDelta::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "Xin chào 🦀");
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        LlmDelta::Usage { usage } if usage.input_tokens == 3 && usage.output_tokens == 2
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        LlmDelta::Stop {
            reason: StopReason::EndTurn
        }
    )));
}

#[tokio::test]
async fn http_error_is_mapped_with_truncated_body() {
    let server = MockServer::start().await;
    let long_message = "x".repeat(2_000);
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error",
            "error": { "type": "invalid_request_error", "message": long_message }
        })))
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("x")];
    let err = provider.chat(request(&messages, &[])).await.unwrap_err();
    match err {
        LlmError::HttpStatus { status, body } => {
            assert_eq!(status, 400);
            assert!(body.contains("đã cắt"), "body phải bị cắt: {body}");
            assert!(body.len() < 1_000, "body cắt phải ngắn: {}", body.len());
        }
        other => panic!("mong HttpStatus, nhận: {other:?}"),
    }
}

#[tokio::test]
async fn rate_limited_without_retry_header_returns_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(json!({
                    "type": "error",
                    "error": { "type": "rate_limit_error", "message": "slow down" }
                }))
                // `Retry-After: 0` giữ `retry_after = Some(0)` (khác None) và test chạy ngay,
                // không phải chờ backoff 1s+2s+4s. Trường hợp `None` (không có header) đã
                // được phủ ở mức unit trong `gives_up_after_max_retries`.
                .insert_header("Retry-After", "0"),
        )
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("x")];
    let err = provider.chat(request(&messages, &[])).await.unwrap_err();
    assert!(
        matches!(
            err,
            LlmError::RateLimited {
                retry_after: Some(delay)
            } if delay.is_zero()
        ),
        "{err:?}"
    );
    // Chính sách mục 5: 429 được thử lại tối đa 3 lần → tổng cộng 4 request.
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        4,
        "429 phải được retry đúng 3 lần rồi mới trả lỗi"
    );
}
