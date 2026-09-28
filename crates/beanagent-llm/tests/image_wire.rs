//! Serialize khối **ảnh** cho cả hai provider (M26).
//!
//! Yêu cầu bắt buộc trong lượt này: ảnh phải tới được model trên **cả** Anthropic
//! lẫn OpenAI-compat, với đúng shape mà mỗi API chấp nhận.
//!
//! * **Anthropic Messages API** — ảnh đi **bên trong** block `tool_result`, content
//!   là mảng block. Đây là dạng chính thức, không cần message phụ.
//! * **OpenAI Chat Completions** — message `role=tool` **bắt buộc** có `content` là
//!   chuỗi, nên ảnh phải đi bằng một message `user` kế tiếp với `content` mảng part.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use beanagent_llm::{ChatRequest, build_anthropic_body, build_openai_body};
use beanagent_types::{ImageBlock, Message, ToolCall};

/// 1.000 ký tự base64 — đủ để mọi so khớp chuỗi bên dưới là có chủ đích.
const DATA: &str = "iVBORw0KGgo=";

fn image() -> ImageBlock {
    ImageBlock::new("image/png", DATA, "abc123").unwrap()
}

fn history() -> Vec<Message> {
    vec![
        Message::user("chụp màn hình giúp tôi"),
        Message::assistant(
            None,
            vec![ToolCall::new(
                "call-1",
                "browser_screenshot",
                serde_json::json!({}),
            )],
        ),
        Message::tool_with_image("call-1", "Ảnh chụp màn hình — 120 KB", image()),
    ]
}

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

#[test]
fn anthropic_puts_image_inside_tool_result_block() {
    let messages = history();
    let body = build_anthropic_body(
        &ChatRequest {
            system: "",
            messages: &messages,
            tools: &[],
            max_tokens: 256,
        },
        "claude-test",
    )
    .expect("dựng body được");

    let msgs = body["messages"].as_array().expect("messages là mảng");
    // Message `user` cuối chứa các block `tool_result`.
    let last = msgs.last().expect("có message cuối");
    let blocks = last["content"].as_array().expect("content là mảng block");
    let result = blocks
        .iter()
        .find(|b| b["type"] == "tool_result")
        .expect("phải có block tool_result");

    assert_eq!(result["tool_use_id"], "call-1");
    // Content của tool_result là mảng chứa block ảnh.
    let content = result["content"]
        .as_array()
        .expect("content tool_result là mảng");
    let image_block = content
        .iter()
        .find(|b| b["type"] == "image")
        .expect("phải có block image");
    assert_eq!(image_block["source"]["type"], "base64");
    assert_eq!(image_block["source"]["media_type"], "image/png");
    assert_eq!(image_block["source"]["data"], DATA);
}

/// Message tool **không** có ảnh phải giữ nguyên dạng chuỗi — hồi quy.
#[test]
fn anthropic_keeps_plain_tool_result_as_string() {
    let messages = vec![
        Message::user("hỏi"),
        Message::assistant(
            None,
            vec![ToolCall::new("c1", "read_file", serde_json::json!({}))],
        ),
        Message::tool("c1", "nội dung file"),
    ];
    let body = build_anthropic_body(
        &ChatRequest {
            system: "",
            messages: &messages,
            tools: &[],
            max_tokens: 256,
        },
        "claude-test",
    )
    .expect("dựng body được");
    let msgs = body["messages"].as_array().expect("messages là mảng");
    let result = &msgs.last().expect("message cuối")["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["content"], "nội dung file");
}

// ---------------------------------------------------------------------------
// OpenAI-compat
// ---------------------------------------------------------------------------

#[test]
fn openai_sends_image_as_a_separate_user_message() {
    let messages = history();
    let body = build_openai_body(
        &ChatRequest {
            system: "",
            messages: &messages,
            tools: &[],
            max_tokens: 256,
        },
        "gpt-test",
    )
    .expect("dựng body được");

    let msgs = body["messages"].as_array().expect("messages là mảng");
    // Message `tool` phải GIỮ content chuỗi (bắt buộc theo spec OpenAI).
    let tool_msg = msgs
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("phải có message role=tool");
    assert!(
        tool_msg["content"].is_string(),
        "content của role=tool phải là CHUỖI, nhận: {}",
        tool_msg["content"]
    );
    // Ảnh nằm ở message `user` ngay sau đó.
    let user_msg = msgs
        .iter()
        .rfind(|m| m["role"] == "user")
        .expect("phải có message role=user chứa ảnh");
    let parts = user_msg["content"]
        .as_array()
        .expect("content là mảng part");
    let part = parts
        .iter()
        .find(|p| p["type"] == "image_url")
        .expect("phải có part image_url");
    assert_eq!(
        part["image_url"]["url"],
        format!("data:image/png;base64,{DATA}")
    );
}

/// Ảnh phải tới **sau** message tool tương ứng, không chen giữa cặp tool.
#[test]
fn openai_image_message_follows_its_tool_result() {
    let messages = history();
    let body = build_openai_body(
        &ChatRequest {
            system: "",
            messages: &messages,
            tools: &[],
            max_tokens: 256,
        },
        "gpt-test",
    )
    .expect("dựng body được");
    let msgs = body["messages"].as_array().expect("messages là mảng");
    let tool_index = msgs
        .iter()
        .position(|m| m["role"] == "tool")
        .expect("có message tool");
    assert!(
        msgs[tool_index + 1]["content"].is_array(),
        "message ngay sau tool result phải chứa ảnh"
    );
}

/// `media_type` lạ trong lịch sử cũ bị thay bằng `image/png` thay vì gửi đi và
/// bị API từ chối (400 ⇒ hỏng cả lượt chat).
#[test]
fn unknown_media_type_falls_back_instead_of_being_sent() {
    let rogue = ImageBlock {
        media_type: "image/svg+xml".to_string(),
        data: DATA.to_string(),
        sha256: "x".to_string(),
    };
    let messages = vec![
        Message::user("hỏi"),
        Message::assistant(
            None,
            vec![ToolCall::new(
                "c1",
                "browser_screenshot",
                serde_json::json!({}),
            )],
        ),
        Message::tool_with_image("c1", "ảnh", rogue),
    ];

    let anthropic = build_anthropic_body(
        &ChatRequest {
            system: "",
            messages: &messages,
            tools: &[],
            max_tokens: 256,
        },
        "claude-test",
    )
    .expect("dựng body được");
    let raw = serde_json::to_string(&anthropic).expect("serialize được");
    assert!(
        !raw.contains("svg+xml"),
        "không được gửi media type lạ lên API"
    );

    let openai = build_openai_body(
        &ChatRequest {
            system: "",
            messages: &messages,
            tools: &[],
            max_tokens: 256,
        },
        "gpt-test",
    )
    .expect("dựng body được");
    let raw = serde_json::to_string(&openai).expect("serialize được");
    assert!(
        !raw.contains("svg+xml"),
        "không được gửi media type lạ lên API"
    );
}
