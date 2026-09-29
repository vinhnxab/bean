//! Kiểm tra provider OpenAI-compat (agents.md mục 20 — M2).
//!
//! Điểm then chốt được phủ: `arguments` là **chuỗi JSON** (gửi & nhận), status lỗi có
//! `error.message`, bỏ Authorization khi không có key (Ollama), và mọi 4xx không retry.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use secrecy::SecretString;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use bean_llm::{
    ChatRequest, LlmError, LlmProvider, OpenAiCompatProvider, build_openai_body,
    parse_openai_response,
};
use bean_types::llm::StopReason;
use bean_types::{LlmDelta, Message, ToolCall, ToolSpec};
use futures_util::StreamExt;

const API_KEY: &str = "k-openai-test";

fn provider_for(server: &MockServer) -> OpenAiCompatProvider {
    OpenAiCompatProvider::new(
        "mo-hinh-test",
        Some(SecretString::from(API_KEY)),
        Some(&format!("{}/v1", server.uri())),
        "OPENAI_API_KEY",
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
fn body_shape_and_tool_arguments_are_string() {
    let assistant = Message::from_response(&bean_types::LlmResponse::with_tool_calls(vec![
        ToolCall::new("c1", "read_file", json!({"path": "a.txt"})),
    ]));
    let messages = [
        Message::user("hỏi"),
        assistant,
        Message::tool("c1", "nội dung"),
    ];
    let body = build_openai_body(&request(&messages, &[]), "mo-hinh-test").unwrap();
    assert_eq!(body["model"], "mo-hinh-test");
    assert_eq!(body["max_tokens"], 256);
    let wire = body["messages"].as_array().unwrap();
    // system + user + assistant + tool = 4.
    assert_eq!(wire.len(), 4);
    assert_eq!(wire[0]["role"], "system");
    assert_eq!(wire[0]["content"], "bạn là trợ lý");
    assert_eq!(wire[2]["role"], "assistant");
    assert_eq!(
        wire[2]["content"],
        Value::Null,
        "chỉ gọi tool → content null"
    );
    let call = &wire[2]["tool_calls"][0];
    assert_eq!(call["id"], "c1");
    assert_eq!(call["type"], "function");
    assert_eq!(call["function"]["name"], "read_file");
    // ⚠️ Điểm mấu chốt: arguments là CHUỖI JSON.
    let args = call["function"]["arguments"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(args).unwrap(),
        json!({"path": "a.txt"})
    );
    // Message tool: role tool + tool_call_id.
    assert_eq!(wire[3]["role"], "tool");
    assert_eq!(wire[3]["tool_call_id"], "c1");
    assert_eq!(wire[3]["content"], "nội dung");
}

#[test]
fn body_tools_use_parameters_and_sanitize() {
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
    let body = build_openai_body(&request(&messages, &tools), "m").unwrap();
    let function = &body["tools"][0]["function"];
    assert_eq!(function["name"], "read_file");
    assert_eq!(function["description"], "Đọc file");
    let schema = &function["parameters"];
    assert_eq!(schema["type"], "object");
    assert!(schema.get("$schema").is_none());
    assert_eq!(schema["properties"]["path"]["type"], "string");
}

#[test]
fn body_empty_messages_is_config_error() {
    let empty: [Message; 0] = [];
    let err = build_openai_body(&request(&empty, &[]), "m").unwrap_err();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
}

// ===== (1b) Parse phản hồi =====

#[test]
fn parse_tool_calls_with_string_arguments() {
    let resp = parse_openai_response(
        r#"{
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 9, "completion_tokens": 4 }
        }"#,
    )
    .unwrap();
    assert!(resp.text.is_none());
    assert_eq!(resp.stop, StopReason::ToolUse);
    assert_eq!(resp.usage.input_tokens, 9);
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "call_1");
    assert_eq!(resp.tool_calls[0].args["path"], "a.txt");
}

#[test]
fn parse_tolerant_of_nonstandard_argument_shapes() {
    // arguments rỗng / null / đã là object (một số server local trả kiểu vậy).
    for (raw, expected) in [
        ("\"\"", json!({})),
        ("null", json!({})),
        ("{\"a\":1}", json!({"a": 1})),
    ] {
        let body = format!(
            r#"{{ "choices": [{{ "message": {{ "tool_calls": [{{ "id": "x", "function": {{ "name": "t", "arguments": {raw} }} }} ] }}, "finish_reason": "tool_calls" }} ] }}"#
        );
        let resp = parse_openai_response(&body).unwrap();
        assert_eq!(resp.tool_calls[0].args, expected, "arguments = {raw}");
    }
}

#[test]
fn parse_invalid_arguments_json_is_decode_error() {
    let body = r#"{ "choices": [{ "message": { "tool_calls": [{ "id": "x", "function": { "name": "t", "arguments": "{sai" } }] } } ] }"#;
    let err = parse_openai_response(body).unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "{err:?}");
}

#[test]
fn parse_missing_function_or_name_is_decode_error() {
    // tool_calls thiếu function.
    let err = parse_openai_response(
        r#"{ "choices": [{ "message": { "tool_calls": [{ "id": "x" }] } }] }"#,
    )
    .unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "{err:?}");
    // function thiếu name.
    let err = parse_openai_response(
        r#"{ "choices": [{ "message": { "tool_calls": [{ "id": "x", "function": { "arguments": "{}" } }] } }] }"#,
    )
    .unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "{err:?}");
}

#[test]
fn parse_finish_reason_mapping() {
    let parse = |finish: &str| {
        parse_openai_response(&format!(
            r#"{{ "choices": [{{ "message": {{ "content": "x" }}, "finish_reason": "{finish}" }}] }}"#
        ))
        .unwrap()
        .stop
    };
    assert_eq!(parse("stop"), StopReason::EndTurn);
    assert_eq!(parse("tool_calls"), StopReason::ToolUse);
    assert_eq!(parse("length"), StopReason::MaxTokens);
    assert_eq!(parse("content_filter"), StopReason::Other);
}

#[test]
fn parse_no_choices_is_decode_error() {
    let err = parse_openai_response(r#"{ "choices": [] }"#).unwrap_err();
    assert!(matches!(err, LlmError::Decode(_)), "{err:?}");
}

// ===== (2) Tích hợp qua wiremock =====

#[tokio::test]
async fn chat_sends_bearer_and_parses_reply() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", format!("Bearer {API_KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "xin chào lại" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 2 }
        })))
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("xin chào")];
    let resp = provider.chat(request(&messages, &[])).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("xin chào lại"));
    assert_eq!(resp.stop, StopReason::EndTurn);
}

#[tokio::test]
async fn chat_stream_parses_sse_text_usage_and_done() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Xin\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" chào\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "stream": true,
            "stream_options": {"include_usage": true}
        })))
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
    assert_eq!(text, "Xin chào");
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
async fn http_error_extracts_error_message() {
    let server = MockServer::start().await;
    let long_message = format!("{}{}", "chi tiết lỗi ", "y".repeat(2_000));
    // 400 = không retry (chính sách mục 5) → đúng 1 request, `.expect(1)` khớp.
    // Việc 5xx được retry 3 lần đã phủ ở `cli.rs::chat_retries_5xx_before_final_error`.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "message": long_message, "type": "invalid_request_error" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("x")];
    let err = provider.chat(request(&messages, &[])).await.unwrap_err();
    match err {
        LlmError::HttpStatus { status, body } => {
            assert_eq!(status, 400);
            assert!(
                body.contains("chi tiết lỗi"),
                "phải lấy error.message: {body}"
            );
            assert!(body.len() < 1_000, "body cắt phải ngắn: {}", body.len());
        }
        other => panic!("mong HttpStatus, nhận: {other:?}"),
    }
}

#[tokio::test]
async fn no_api_key_and_no_base_url_is_config_error() {
    let err = OpenAiCompatProvider::new("m", None, None, "OPENAI_API_KEY").unwrap_err();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
    // Key rỗng coi như không có key.
    let err = OpenAiCompatProvider::new("m", Some(SecretString::from("")), None, "OPENAI_API_KEY")
        .unwrap_err();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
}

/// Lỗi 401 khi Bean **không gửi** header `Authorization` phải nói thẳng tên biến
/// cần đặt. Trước đây người dùng chỉ thấy câu mập mờ của hãng
/// (*"No cookie auth credentials found"*) và phải tự dịch từng chữ.
#[tokio::test]
async fn unauthorized_without_key_names_the_env_var_to_set() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "No cookie auth credentials found" }
        })))
        .mount(&server)
        .await;

    let provider = OpenAiCompatProvider::new(
        "m",
        None,
        Some(&format!("{}/v1", server.uri())),
        "OPENROUTER_API_KEY",
    )
    .unwrap();
    let messages = [Message::user("x")];

    let err = provider.chat(request(&messages, &[])).await.unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
    assert!(
        text.contains("OPENROUTER_API_KEY"),
        "phải nêu đúng tên biến cần đặt, nhận được: {text}"
    );
    // Giữ luôn phản hồi gốc của hãng: nó có thể là manh mối khi nguyên nhân
    // không phải key.
    assert!(text.contains("No cookie auth credentials"), "{text}");
}

/// Ngược lại: **đã gửi key** mà vẫn 401 thì nguyên nhân là key sai/hết hạn, KHÔNG
/// phải thiếu key. Nếu ở đây cũng quy về "thiếu biến môi trường" thì Bean sẽ
/// dẫn người dùng đi sai hướng — họ sẽ export lại một key vốn đã có.
#[tokio::test]
async fn unauthorized_with_key_keeps_vendor_error_instead_of_blaming_missing_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "message": "Key is expired" }
        })))
        .mount(&server)
        .await;

    let provider = provider_for(&server);
    let messages = [Message::user("x")];

    let err = provider.chat(request(&messages, &[])).await.unwrap_err();
    assert!(
        matches!(err, LlmError::HttpStatus { status: 401, .. }),
        "phải giữ dạng HttpStatus, nhận: {err:?}"
    );
    assert!(!err.to_string().contains("chưa được đặt"), "{err}");
}

/// 403 với cùng lý do: nhiều hãng trả 403 thay vì 401 khi thiếu xác thực.
#[tokio::test]
async fn forbidden_without_key_also_names_the_env_var() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "error": {} })))
        .mount(&server)
        .await;

    let provider = OpenAiCompatProvider::new(
        "m",
        None,
        Some(&format!("{}/v1", server.uri())),
        "OPENROUTER_API_KEY",
    )
    .unwrap();
    let messages = [Message::user("x")];

    let err = provider.chat(request(&messages, &[])).await.unwrap_err();
    assert!(err.to_string().contains("OPENROUTER_API_KEY"), "{err}");
}

#[tokio::test]
async fn no_api_key_with_base_url_sends_no_authorization() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "ok" }, "finish_reason": "stop" }]
        })))
        .mount(&server)
        .await;

    let provider = OpenAiCompatProvider::new(
        "m",
        None,
        Some(&format!("{}/v1", server.uri())),
        "OPENAI_API_KEY",
    )
    .unwrap();
    let messages = [Message::user("x")];
    let resp = provider.chat(request(&messages, &[])).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("ok"));
    // Không có header Authorization nào được gửi.
    let requests: &[Request] = &server.received_requests().await.unwrap();
    assert!(
        !requests[0].headers.contains_key("authorization"),
        "Ollama-style không được gửi Authorization"
    );
}
