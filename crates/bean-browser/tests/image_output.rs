//! Kiểm chứng tool trả **ảnh** dùng kiểu nội dung mới (M26).
//!
//! Không cần Chrome thật: test ở đây chốt **hợp đồng kiểu** — `Tool::call_rich`
//! mặc định bọc `call` thành `Text`, và `Message::tool_with_image` mang khối ảnh
//! qua được vòng lặp agent mà không bị cắt theo ký tự.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use bean_browser::build_tools;
use bean_tools::{Tool, ToolCtx, ToolError, ToolOutput};
use bean_types::config::BrowserConfig;
use bean_types::{ImageBlock, Message, Risk, ToolSpec};

/// Tool chỉ trả văn bản — dùng để chứng minh `call_rich` **mặc định** bọc `Text`.
struct TextOnly;

#[async_trait]
impl Tool for TextOnly {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("text_only", "tool chỉ trả text", serde_json::json!({}))
    }
    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Safe
    }
    async fn call(&self, _ctx: &ToolCtx, _args: serde_json::Value) -> Result<String, ToolError> {
        Ok("kết quả văn bản".to_string())
    }
}

fn ctx() -> ToolCtx {
    use bean_security::CapWorkspace;
    use bean_types::SessionId;
    use tokio_util::sync::CancellationToken;
    let dir = tempfile::tempdir().expect("tạo thư mục tạm");
    let leaked: &'static std::path::Path = Box::leak(dir.path().to_path_buf().into_boxed_path());
    ToolCtx::for_project(
        Arc::new(CapWorkspace::open(leaked.to_path_buf()).expect("mở workspace")),
        SessionId::new(1),
        CancellationToken::new(),
        Arc::new(AtomicBool::new(false)),
    )
}

#[tokio::test]
async fn call_rich_defaults_to_text_for_existing_tools() {
    // Đây là bằng chứng phạm vi thay đổi hẹp: 25+ tool cũ không cần sửa gì mà vẫn
    // đi qua `call_rich` của agent loop với hành vi **y hệt** trước đây.
    let out = TextOnly
        .call_rich(&ctx(), serde_json::json!({}))
        .await
        .expect("tool chạy được");
    assert_eq!(out, ToolOutput::Text("kết quả văn bản".to_string()));
    assert!(!out.is_image());
    assert_eq!(out.as_text(), "kết quả văn bản");
}

#[tokio::test]
async fn screenshot_tool_refuses_plain_call_and_requires_call_rich() {
    // `browser_screenshot` trả ảnh; gọi nhầm `call` phải báo lỗi rõ ràng thay vì
    // âm thầm trả chuỗi rỗng khiến model tưởng chụp xong.
    let dir = tempfile::tempdir().expect("tạo thư mục tạm");
    let leaked: &'static std::path::Path = Box::leak(dir.path().to_path_buf().into_boxed_path());
    let settings = BrowserConfig {
        enabled: true,
        ..BrowserConfig::default()
    };
    let (tools, _session) = build_tools(&settings, leaked);
    let tool = tools
        .iter()
        .find(|t| t.spec().name == "browser_screenshot")
        .expect("có tool screenshot");
    let error = tool
        .call(&ctx(), serde_json::json!({}))
        .await
        .expect_err("phải báo lỗi khi gọi nhầm `call`");
    assert!(
        error.to_string().contains("call_rich"),
        "thông điệp phải chỉ ra đường đúng: {error}"
    );
}

/// `Message::tool_with_image` mang khối ảnh qua vòng lặp mà **không** bị cắt ký tự.
#[test]
fn image_message_keeps_full_payload_and_caption() {
    // Base64 giả 100.000 ký tự — nếu đi qua `truncate_output` (trần 20.000) thì ảnh
    // sẽ hỏng. Đây là lý do agent loop tách nhánh ảnh khỏi nhánh text.
    let data = "A".repeat(100_000);
    let image = ImageBlock::new("image/png", data.clone(), "deadbeef").expect("media type hợp lệ");
    let message = Message::tool_with_image("call-1", "Ảnh chụp màn hình", image);

    assert_eq!(message.text.as_deref(), Some("Ảnh chụp màn hình"));
    let block = message.image.as_ref().expect("phải có khối ảnh");
    assert_eq!(block.data.len(), data.len(), "payload ảnh phải giữ nguyên");
    assert_eq!(block.media_type, "image/png");
    assert_eq!(block.sha256, "deadbeef");
    assert!(!message.is_error);
}

/// `media_type` lạ bị chặn ở tầng dựng khối (fail-closed trước khi tới provider).
#[test]
fn disallowed_media_type_is_rejected_at_construction() {
    for bad in ["image/svg+xml", "text/html", "application/javascript", ""] {
        let result = ImageBlock::new(bad, "AAAA", "hash");
        assert!(result.is_err(), "media_type `{bad}` phải bị từ chối");
    }
    for good in ["image/png", "image/jpeg", "image/webp"] {
        assert!(
            ImageBlock::new(good, "AAAA", "hash").is_ok(),
            "media_type `{good}` phải được chấp nhận"
        );
    }
}

/// Message không có ảnh serialize **không** kém trường `image` — giữ tương thích
/// với mọi message đã ghi trước M26.
#[test]
fn message_without_image_serializes_without_the_field() {
    let plain = Message::tool("c1", "kết quả");
    let json = serde_json::to_string(&plain).expect("serialize được");
    assert!(
        !json.contains("\"image\""),
        "message không có ảnh không được sinh trường `image`: {json}"
    );
    // Và vẫn deserialize được khi đọc lại.
    let back: Message = serde_json::from_str(&json).expect("deserialize được");
    assert!(back.image.is_none());
}
