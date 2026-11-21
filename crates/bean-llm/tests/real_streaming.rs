//! Kiểm chứng streaming thật (M17) — opt-in, KHÔNG chạy trong `make check`.
//!
//! Chạy tay (key chỉ đọc từ env, không bao giờ in):
//! `set -a; . ~/.bean/secrets.env; set +a; cargo test -p bean-llm --test real_streaming -- --ignored --nocapture`
//!
//! Những gì đo trên dây thật (NVIDIA, OpenAI-compat):
//! 1. `stream: true` + `stream_options.include_usage` được chấp nhận (không 400).
//! 2. Text về **nhiều delta** liên tiếp — nền cho chữ hiện dần trong UI.
//! 3. `include_usage` thực sự trả usage — ngân sách token M21.7 đếm qua đường stream.
//! 4. Tool call stream dạng delta, ghép `arguments_delta` thành JSON hợp lệ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use futures_util::StreamExt;
use secrecy::SecretString;
use serde_json::{Value, json};

use bean_llm::{ChatRequest, LlmProvider, OpenAiCompatProvider};
use bean_types::{LlmDelta, LlmToolCallDelta, Message, ToolSpec};

const BASE_URL: &str = "https://integrate.api.nvidia.com/v1";
const MODEL: &str = "z-ai/glm-5.3-flash";
const KEY_ENV: &str = "NVIDIA_KEY";

fn real_provider() -> OpenAiCompatProvider {
    let key = std::env::var(KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("chưa đặt {KEY_ENV} — nạp secrets.env rồi chạy lại"));
    OpenAiCompatProvider::new(
        MODEL,
        Some(SecretString::from(key)),
        Some(BASE_URL),
        KEY_ENV,
    )
    .unwrap()
}

fn request<'a>(messages: &'a [Message], tools: &'a [ToolSpec]) -> ChatRequest<'a> {
    ChatRequest {
        system: "bạn là trợ lý kiểm thử",
        messages,
        tools,
        max_tokens: 4096,
    }
}

#[tokio::test]
#[ignore = "gọi NVIDIA thật — cần mạng + NVIDIA_KEY"]
async fn real_stream_returns_multiple_text_deltas_with_usage() {
    let provider = real_provider();
    let messages = [Message::user(
        "Đếm từ 1 đến 5, mỗi số nằm trên một dòng riêng, không viết thêm gì khác.",
    )];
    let mut stream = provider
        .chat_stream(request(&messages, &[]))
        .await
        .expect("mở stream thất bại — kiểm tra mạng/key");

    let mut text_delta_count = 0usize;
    let mut text = String::new();
    let mut usage = None;
    let mut stop = None;
    while let Some(item) = stream.next().await {
        match item.expect("delta giữa dòng lỗi") {
            LlmDelta::Text { text: chunk } => {
                if !chunk.is_empty() {
                    text_delta_count += 1;
                    text.push_str(&chunk);
                }
            }
            LlmDelta::Usage { usage: raw } => usage = Some(raw),
            LlmDelta::Stop { reason } => stop = Some(reason),
            LlmDelta::ToolCall(_) => panic!("không cấp tool mà model vẫn gọi tool"),
        }
    }

    assert!(
        text_delta_count >= 3,
        "chỉ nhận {text_delta_count} delta text — stream không tiến triển dần (về khối duy nhất?)"
    );
    assert!(
        text.contains('1') && text.contains('5'),
        "nội dung sai: {text}"
    );
    let usage = usage.expect(
        "không nhận Usage delta — stream_options.include_usage không có tác dụng trên endpoint này",
    );
    assert!(
        usage.input_tokens > 0 && usage.output_tokens > 0,
        "usage rỗng: {:?}",
        usage
    );
    assert!(
        stop.is_some(),
        "không nhận Stop delta — finish_reason bị mất"
    );
    println!(
        "text_deltas={text_delta_count} chars={} usage={}+{} stop={:?}",
        text.chars().count(),
        usage.input_tokens,
        usage.output_tokens,
        stop
    );
}

#[tokio::test]
#[ignore = "gọi NVIDIA thật — cần mạng + NVIDIA_KEY"]
async fn real_stream_assembles_tool_call_arguments() {
    let provider = real_provider();
    let tools = [ToolSpec::new(
        "read_file",
        "Đọc nội dung một file trong workspace theo đường dẫn tương đối.",
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
    )];
    let messages = [Message::user(
        "Dùng tool read_file để đọc file 'ghi-chu.md'. Chỉ gọi tool, không giải thích.",
    )];
    let mut stream = provider
        .chat_stream(request(&messages, &tools))
        .await
        .expect("mở stream thất bại — kiểm tra mạng/key");

    // Ghép delta theo index: (id, name, arguments).
    let mut calls: BTreeMap<usize, (Option<String>, Option<String>, String)> = BTreeMap::new();
    while let Some(item) = stream.next().await {
        match item.expect("delta giữa dòng lỗi") {
            LlmDelta::ToolCall(LlmToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            }) => {
                let entry = calls.entry(index).or_default();
                if id.is_some() {
                    entry.0 = id;
                }
                if name.is_some() {
                    entry.1 = name;
                }
                if let Some(args) = arguments_delta {
                    entry.2.push_str(&args);
                }
            }
            LlmDelta::Stop { .. } | LlmDelta::Usage { .. } | LlmDelta::Text { .. } => {}
        }
    }

    assert!(!calls.is_empty(), "model không gọi tool nào qua stream");
    let (id, name, args) = calls.values().next().unwrap();
    assert!(id.is_some(), "tool call thiếu id sau khi ghép delta");
    assert_eq!(name.as_deref(), Some("read_file"));
    let parsed: Value =
        serde_json::from_str(args).expect("arguments ghép từ delta không phải JSON");
    assert_eq!(parsed["path"], "ghi-chu.md", "arguments sai: {args}");
    println!("tool_call id={:?} args={args}", id.as_deref());
}
