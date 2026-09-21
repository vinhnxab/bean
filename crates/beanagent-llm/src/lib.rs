//! # BeanAgent-llm
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

use async_trait::async_trait;
use beanagent_types::{LlmResponse, Message, ToolSpec};
use std::fmt;

/// Yêu cầu gửi tới provider cho một lượt gọi (agents.md mục 5).
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    /// System prompt (đã dựng ở `BeanAgent-core::agent::prompt`).
    pub system: &'a str,
    /// Lịch sử hội thoại trong ngân sách token, đã cắt ở ranh giới an toàn.
    pub messages: &'a [Message],
    /// Tool được phép gọi trong lượt này.
    pub tools: &'a [ToolSpec],
    /// Trần token cho phần sinh ra.
    pub max_tokens: u32,
}

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

    /// Tên provider, dùng cho log và `/api/status`.
    fn name(&self) -> &'static str;
}
