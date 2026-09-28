//! # bean-llm
//!
//! Trait [`LlmProvider`] và các cài đặt provider (agents.md mục 5, 21).
//!
//! * **M1**: trait + [`FakeProvider`].
//! * **M2**: [`AnthropicProvider`] (Messages API) và [`OpenAiCompatProvider`] (Chat
//!   Completions) — tự viết trên `reqwest 0.13` + `serde`, kèm [`retry_with_backoff`]
//!   và `wiremock` cho test.
//! * **M17**: thêm `chat_stream` trả `Stream<Item = LlmDelta>` (streaming token qua WebSocket).
//!
//! Mọi cài đặt phải chuyển đổi `Message` ⇄ định dạng riêng của API, gồm cả cặp
//! `tool_use`/`tool_result` và cờ `is_error`.
//!
//! Nguyên tắc (xem `docs/decisions.md` D6.1): mỗi lượt `chat` phát đúng **một** HTTP
//! request; retry/backoff nằm ở [`retry_with_backoff`] để test đo đếm được và M17 không
//! phải nhân bản logic.
#![forbid(unsafe_code)]

mod anthropic;
mod error;
mod factory;
mod fake;
mod http;
mod openai_compat;
mod retry;
mod schema;
mod stream;

pub use anthropic::AnthropicProvider;
pub use error::LlmError;
pub use factory::build_provider;
pub use fake::{FakeProvider, FakeScript};
pub use http::truncate_body;
pub use openai_compat::OpenAiCompatProvider;
pub use retry::{MAX_RETRIES, retry_with_backoff};

// Hàm wire (dựng body / parse phản hồi) re-export để test tích hợp và để đọc trực tiếp
// không cần phải qua `Provider` (các module là private).
pub use anthropic::{
    build_request_body as build_anthropic_body, parse_response as parse_anthropic_response,
};
pub use openai_compat::{
    build_request_body as build_openai_body, parse_response as parse_openai_response,
};

use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use bean_types::{LlmDelta, LlmResponse, Message, ToolSpec};
use futures_core::Stream;

/// Yêu cầu gửi tới provider cho một lượt gọi (agents.md mục 5).
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    /// System prompt (đã dựng ở `bean-core::agent::prompt`).
    pub system: &'a str,
    /// Lịch sử hội thoại trong ngân sách token, đã cắt ở ranh giới an toàn.
    pub messages: &'a [Message],
    /// Tool được phép gọi trong lượt này.
    pub tools: &'a [ToolSpec],
    /// Trần token cho phần sinh ra.
    pub max_tokens: u32,
}

/// Stream delta do provider sở hữu; mỗi item có thể lỗi nếu kết nối giữa chừng hỏng.
pub type LlmStream = Pin<Box<dyn Stream<Item = Result<LlmDelta, LlmError>> + Send + 'static>>;

/// Nhà cung cấp LLM.
///
/// `Debug` là bắt buộc để mọi chỗ giữ `Arc<dyn LlmProvider>` đều in được mà **không lộ
/// secret** (mỗi cài đặt tự bảo đảm `Debug` đã che API key — agents.md mục 15.6).
#[async_trait]
pub trait LlmProvider: Send + Sync + fmt::Debug {
    /// Thực hiện một lượt chat **không** streaming.
    ///
    /// # Errors
    /// Trả [`LlmError`] khi mạng/cấu hình/parse lỗi. Lỗi tool **không** đi qua đường này
    /// (nó được biến thành message `Tool` với `is_error = true` trong agent loop).
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError>;

    /// Thực hiện một lượt chat streaming.
    ///
    /// Mặc định gom một response non-stream thành các delta tối thiểu. Provider HTTP M17
    /// override để đọc SSE; provider/test cũ chỉ implement [`LlmProvider::chat`] vẫn chạy.
    ///
    /// # Errors
    /// Giống [`LlmProvider::chat`].
    async fn chat_stream(&self, req: ChatRequest<'_>) -> Result<LlmStream, LlmError> {
        let response = self.chat(req).await?;
        Ok(response_to_stream(response))
    }

    /// Chat bằng model override cho request này. Mặc định giữ provider hiện tại;
    /// provider HTTP override để `/model` áp dụng cho run kế tiếp.
    ///
    /// # Errors
    /// Giống [`LlmProvider::chat`].
    async fn chat_with_model(
        &self,
        req: ChatRequest<'_>,
        _model: &str,
    ) -> Result<LlmResponse, LlmError> {
        self.chat(req).await
    }

    /// Chat streaming bằng model override cho request này.
    ///
    /// # Errors
    /// Giống [`LlmProvider::chat_stream`].
    async fn chat_stream_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmStream, LlmError> {
        if model.is_empty() {
            return self.chat_stream(req).await;
        }
        let response = self.chat_with_model(req, model).await?;
        Ok(response_to_stream(response))
    }

    /// Tên provider, dùng cho log và `/api/status`.
    fn name(&self) -> &'static str;
}

fn response_to_stream(response: LlmResponse) -> LlmStream {
    use futures_util::stream;

    let mut deltas = Vec::new();
    if let Some(text) = response.text.filter(|text| !text.is_empty()) {
        deltas.push(Ok(LlmDelta::Text { text }));
    }
    for (index, call) in response.tool_calls.into_iter().enumerate() {
        deltas.push(Ok(LlmDelta::ToolCall(bean_types::LlmToolCallDelta {
            index,
            id: Some(call.id),
            name: Some(call.name),
            arguments_delta: Some(call.args.to_string()),
        })));
    }
    deltas.push(Ok(LlmDelta::Usage {
        usage: response.usage,
    }));
    deltas.push(Ok(LlmDelta::Stop {
        reason: response.stop,
    }));
    Box::pin(stream::iter(deltas))
}
