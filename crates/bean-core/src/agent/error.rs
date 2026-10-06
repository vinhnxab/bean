//! Lỗi của vòng lặp agent — tách riêng để `impl`/`From` không lẫn vào vòng lặp.

use super::*;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("lỗi lưu trữ: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("lỗi provider (LLM): {0}")]
    Llm(String),
    /// Người dùng huỷ run (`CancellationToken` tường minh — mục 10).
    #[error("run đã bị huỷ")]
    Cancelled,
    #[error("tool `{}' thất bại liên tiếp — hãy thử cách khác", 0)]
    RepeatFailure(String),
}

impl AgentError {
    /// Mã lỗi ổn định cho adapter, không chứa nội dung bí mật.
    ///
    /// Đặt cạnh [`RouterError`](crate::router::RouterError) như một **method** để
    /// biểu đồ "loại lỗi ⇒ mã ổn định" nằm ngay trên kiểu lỗi, không phải ở
    /// module khác — trước đây có hai bản `match` trùng nhau dễ lệch khi thêm biến thể.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Store(_) => "store",
            Self::Llm(_) => "llm",
            Self::Cancelled => "cancelled",
            Self::RepeatFailure(_) => "loop_guard",
        }
    }
}

impl From<LlmError> for AgentError {
    fn from(err: LlmError) -> Self {
        AgentError::Llm(err.to_string())
    }
}
