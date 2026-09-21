//! Định dạng message trung lập với provider (agents.md mục 5).
//!
//! Bất biến quan trọng: một `Assistant` có `tool_calls` luôn phải đi kèm các `Tool` result
//! **đủ và đúng thứ tự** (`tool_call_id` khớp). Mọi thao tác cắt lịch sử trong
//! `BeanAgent-memory` phải giữ nguyên cặp này, nếu không API sẽ trả lỗi 400
//! (agents.md mục 8.3, 22.1).

use serde::{Deserialize, Serialize};

use crate::llm::LlmResponse;

/// Vai trò của message.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Người dùng.
    User,
    /// Model (có thể kèm `tool_calls`).
    Assistant,
    /// Kết quả tool trả lại cho model.
    Tool,
}

impl Role {
    /// Chuỗi dùng trong cột `messages.role` của SQLite.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// Một yêu cầu gọi tool do model phát ra.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolCall {
    /// Id do provider sinh; phải được giữ nguyên khi gửi lại tool result.
    pub id: String,
    /// Tên tool, khớp `ToolSpec::name`.
    pub name: String,
    /// Tham số thô (`serde_json::Value`) — là đầu vào **không tin cậy**, phải validate trước khi dùng.
    pub args: serde_json::Value,
}

impl ToolCall {
    /// Tạo một tool call.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
        }
    }
}

/// Một message trong hội thoại.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub text: Option<String>,
    /// Chỉ có ý nghĩa với `Assistant`.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Chỉ có ý nghĩa với `Tool`.
    pub tool_call_id: Option<String>,
    /// Chỉ có ý nghĩa với `Tool`: đây là lỗi (để model đọc và tự sửa, agents.md mục 6).
    #[serde(default)]
    pub is_error: bool,
}

impl Message {
    /// Message của người dùng.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            is_error: false,
        }
    }

    /// Message của model (có thể vừa có text vừa có `tool_calls`).
    #[must_use]
    pub fn assistant(text: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            text,
            tool_calls,
            tool_call_id: None,
            is_error: false,
        }
    }

    /// Kết quả tool thành công.
    #[must_use]
    pub fn tool(tool_call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            text: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            is_error: false,
        }
    }

    /// Kết quả tool thất bại (kể cả bị huỷ, timeout, sai tham số).
    #[must_use]
    pub fn tool_error(tool_call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::tool(tool_call_id, text)
        }
    }

    /// Chuyển một `LlmResponse` thành message của assistant để ghi vào lịch sử.
    #[must_use]
    pub fn from_response(resp: &LlmResponse) -> Self {
        Self::assistant(resp.text.clone(), resp.tool_calls.clone())
    }

    /// Văn bản dùng cho cột `text_for_search` (FTS5, agents.md mục 8.1).
    ///
    /// Gộp phần text nhìn thấy được và tên/đối số tool để tìm kiếm được cả hành động.
    #[must_use]
    pub fn text_for_search(&self) -> String {
        let mut out = self.text.clone().unwrap_or_default();
        for call in &self.tool_calls {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[tool:{}] {}", call.name, call.args));
        }
        out
    }
}
