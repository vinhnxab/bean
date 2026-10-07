//! Lỗi của vòng lặp agent — tách riêng để `impl`/`From` không lẫn vào vòng lặp.
//!
//! # SOLID Principles Applied
//!
//! - **S** - Single Responsibility: Mỗi loại lỗi có một trách nhiệm cụ thể
//! - **O** - Open/Closed: Thêm lỗi mới không cần sửa existing code
//! - **D** - Dependency Inversion: Lỗi phụ thuộc vào traits, không vào implementations

use super::*;

/// Lỗi về build context cho agent
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("không thể build context: {0}")]
    BuildFailed(String),
    #[error("ngân sách token đã đạt giới hạn")]
    BudgetExceeded,
}

/// Lỗi về thực thi tool
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("tool `{0}` hết giờ sau {1:?}")]
    Timeout(String, std::time::Duration),
    #[error("đã đạt giới hạn số bước ({0})")]
    MaxSteps(usize),
    #[error("tool `{0}` không tồn tại trong registry")]
    ToolNotFound(String),
}

/// Lỗi về policy/routing
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("không được phép gọi tool `{0}`")]
    PermissionDenied(String),
    #[error("tool `{0}` yêu cầu xác nhận")]
    ConfirmationRequired(String),
}

/// Lỗi về tool execution
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool `{tool}` executed với tham số sai: {error}")]
    InvalidArgs { tool: String, error: String },
    #[error("tool `{0}` thất bại: {1}")]
    ExecutionFailed(String, String),
    #[error("tool `{0}` trả về output quá dài, đã cắt {1} ký tự")]
    OutputTruncated(String, usize),
}

/// Lỗi chính của vòng lặp agent — đã phân tách theo SRP
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("lỗi lưu trữ: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("lỗi context: {0}")]
    Context(#[from] ContextError),
    #[error("lỗi thực thi: {0}")]
    Execution(#[from] ExecutionError),
    #[error("lỗi policy: {0}")]
    Policy(#[from] PolicyError),
    #[error("lỗi tool: {0}")]
    Tool(#[from] ToolError),
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
            Self::Context(_) => "context",
            Self::Execution(_) => "execution",
            Self::Policy(_) => "policy",
            Self::Tool(_) => "tool",
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

/// Helper functions cho error handling
impl AgentError {
    /// Kiểm tra nếu error là do huỷ run
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Kiểm tra nếu error là do policy (permission denied)
    pub fn is_policy_error(&self) -> bool {
        matches!(self, Self::Policy(_))
    }

    /// Kiểm tra nếu error là do tool
    pub fn is_tool_error(&self) -> bool {
        matches!(self, Self::Tool(_))
    }

    /// Wrap một error khác thành AgentError
    pub fn wrap<T, E: Into<AgentError>>(err: E) -> Result<T, AgentError> {
        Err(err.into())
    }
}
