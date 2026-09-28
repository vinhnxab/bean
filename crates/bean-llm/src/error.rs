//! Lỗi của lớp gọi LLM.
//!
//! `is_retryable()` là nguồn sự thật duy nhất cho quyết định thử lại (agents.md mục 5):
//! chỉ 429/5xx/lỗi mạng/timeout được thử lại, tối đa 3 lần, có backoff + jitter và tôn trọng
//! `retry-after`. Mọi 4xx khác **không** thử lại.

use std::time::Duration;

/// Lỗi khi gọi provider.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// Provider trả HTTP 4xx/5xx kèm body (đã cắt ngắn, đã redact).
    #[error("provider trả HTTP {status}: {body}")]
    HttpStatus {
        /// Mã trạng thái HTTP.
        status: u16,
        /// Body đã cắt ngắn.
        body: String,
    },
    /// Provider giới hạn tần suất (HTTP 429). Có thể kèm `retry-after`.
    #[error("provider giới hạn tần suất (429){}", .retry_after.map_or(String::new(), |d| format!(", retry-after={}s", d.as_secs())))]
    RateLimited {
        /// Giá trị `retry-after` đã parse (nếu có).
        retry_after: Option<Duration>,
    },
    /// Không kết nối được / lỗi mạng.
    #[error("lỗi mạng: {0}")]
    Transport(String),
    /// Quá thời gian chờ.
    #[error("provider bị timeout")]
    Timeout,
    /// Không phân tích được JSON trả về (kể cả `arguments` của tool call).
    #[error("không xử lý được phản hồi của provider: {0}")]
    Decode(String),
    /// Cấu hình provider không hợp lệ.
    #[error("cấu hình provider không hợp lệ: {0}")]
    Config(String),
    /// Kịch bản `--fake-llm` không hợp lệ.
    #[error("kịch bản giả không hợp lệ: {0}")]
    FakeScript(String),
    /// Kịch bản `--fake-llm` đã hết response.
    #[error("kịch bản giả đã hết: cần response #{requested} nhưng chỉ có {available}")]
    FakeScriptExhausted {
        /// Chỉ số response đang được yêu cầu (1-based).
        requested: usize,
        /// Số response có trong kịch bản.
        available: usize,
    },
    /// Lượt gọi bị huỷ (người dùng bấm Dừng / Ctrl-C).
    #[error("lượt gọi đã bị huỷ")]
    Cancelled,
    /// Lỗi nội bộ khác.
    #[error("lỗi nội bộ: {0}")]
    Internal(String),
}

impl LlmError {
    /// Có nên thử lại không? (agents.md mục 5 — chỉ 429/5xx/lỗi mạng/timeout)
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Timeout | Self::RateLimited { .. } => true,
            Self::HttpStatus { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }

    /// Giá trị `retry-after` do provider yêu cầu, nếu có.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }
}
