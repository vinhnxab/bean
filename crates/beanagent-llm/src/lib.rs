//! # BeanAgent-llm
//!
//! Trait [`LlmProvider`] và các cài đặt provider (agents.md mục 5, 21).
//!
//! * **M1**: trait + [`FakeProvider`].
//! * **M2**: `anthropic` (Messages API) và `openai_compat` (Chat Completions) — tự viết trên
//!   `reqwest 0.13` + `serde`, kèm retry/backoff và `wiremock` cho test.
//! * **M17**: thêm `chat_stream` trả `Stream<Item = LlmDelta>` (streaming token qua WebSocket).
//!
//! Mọi cài đặt phải chuyển đổi `Message` ⇄ định dạng riêng của API, gồm cả cặp
//! `tool_use`/`tool_result` và cờ `is_error`.
#![forbid(unsafe_code)]

mod error;
mod fake;

pub use error::LlmError;
pub use fake::{FakeProvider, FakeScript};

use async_trait::async_trait;
use beanagent_types::{LlmResponse, Message, ToolSpec};

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
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Thực hiện một lượt chat **không** streaming.
    ///
    /// # Errors
    /// Trả [`LlmError`] khi mạng/cấu hình/parse lỗi. Lỗi tool **không** đi qua đường này
    /// (nó được biến thành message `Tool` với `is_error = true` trong agent loop).
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError>;

    /// Tên provider, dùng cho log và `/api/status`.
    fn name(&self) -> &'static str;
}
