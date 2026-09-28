//! Kiểu trả về của LLM provider (agents.md mục 5).

use serde::{Deserialize, Serialize};

use crate::message::ToolCall;

/// Số token đã dùng cho một lượt gọi.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl Usage {
    /// Tổng token của lượt gọi (dùng cho ngân sách ngày, agents.md mục 15.9).
    #[must_use]
    pub const fn total(self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Lý do provider dừng sinh.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model trả lời xong.
    EndTurn,
    /// Model muốn gọi tool — vòng lặp phải thực thi rồi gọi lại.
    ToolUse,
    /// Chạm trần token.
    MaxTokens,
    /// Giá trị khác mà provider trả về (đã log lại).
    Other,
}

impl Default for StopReason {
    /// Mặc định `Other` (không phải `EndTurn`) để script `--fake-llm` thiếu trường `stop`
    /// không vô tình bị hiểu là "đã trả lời xong".
    fn default() -> Self {
        Self::Other
    }
}

impl StopReason {
    /// Có phải model muốn gọi tool?
    #[must_use]
    pub const fn is_tool_use(self) -> bool {
        matches!(self, Self::ToolUse)
    }
}

/// Một phần tool call nhận được từ stream.
///
/// `arguments_delta` là **một đoạn JSON**, không phải JSON hoàn chỉnh. Agent loop
/// gom theo `index` trước khi parse thành [`ToolCall`](crate::message::ToolCall).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LlmToolCallDelta {
    /// Vị trí tool call trong danh sách của provider.
    pub index: usize,
    /// ID có thể chỉ xuất hiện ở chunk đầu.
    #[serde(default)]
    pub id: Option<String>,
    /// Tên function có thể chỉ xuất hiện ở chunk đầu.
    #[serde(default)]
    pub name: Option<String>,
    /// Đoạn JSON arguments tăng dần.
    #[serde(default)]
    pub arguments_delta: Option<String>,
}

/// Một delta trung lập từ LLM streaming.
///
/// Provider chỉ cần phân tích SSE riêng; agent loop chỉ biết enum này nên không phụ
/// thuộc wire format của Anthropic/OpenAI.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmDelta {
    /// Văn bản mới sinh.
    Text {
        /// Đoạn text có thể ghép trực tiếp.
        text: String,
    },
    /// Tool call mới hoặc phần bổ sung của một tool call.
    ToolCall(LlmToolCallDelta),
    /// Lý do provider kết thúc lượt sinh.
    Stop {
        /// Stop reason đã chuẩn hoá.
        reason: StopReason,
    },
    /// Usage cập nhật. Giá trị là snapshot tích luỹ, không phải delta phải cộng.
    Usage {
        /// Snapshot token usage.
        usage: Usage,
    },
}

/// Kết quả một lượt gọi LLM.
///
/// `Default` sinh ra response "rỗng": không text, không tool call, `stop = Other`
/// (an toàn hơn `EndTurn`: thiếu trường trong JSON không bị hiểu là "đã trả lời xong"),
/// `usage` bằng 0.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct LlmResponse {
    /// Phần văn bản (có thể rỗng khi chỉ gọi tool).
    pub text: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub stop: StopReason,
    #[serde(default)]
    pub usage: Usage,
}

impl LlmResponse {
    /// Response chỉ có văn bản: lượt này kết thúc (`EndTurn`), không gọi tool.
    ///
    /// Lưu ý: `stop` là `EndTurn` chỉ khi dùng hàm này. Nếu nạp từ JSON thiếu trường `stop`
    /// thì giá trị là `Other` (an toàn hơn `EndTurn` — xem `impl Default for StopReason`).
    #[must_use]
    pub fn text_only(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            stop: StopReason::EndTurn,
            ..Self::default()
        }
    }

    /// Response yêu cầu gọi tool.
    #[must_use]
    pub fn with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            stop: StopReason::ToolUse,
            ..Self::default()
        }
    }
}
