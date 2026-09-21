//! Mô tả tool và mức rủi ro (agents.md mục 7).

use serde::{Deserialize, Serialize};

/// Mức rủi ro quyết định cách xin xác nhận (agents.md mục 7.2).
///
/// Định nghĩa ở crate `types` (agents.md mục 7.1 đặt ở `BeanAgent-tools`) để `tools`, `core`
/// và `web` dùng chung một kiểu — xem `docs/decisions.md` D5.9.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Chạy thẳng.
    Safe,
    /// Hỏi mỗi lần, có tuỳ chọn "cho phép tool này trong phiên".
    Confirm,
    /// Luôn hỏi, **không** có tuỳ chọn cho phép cả phiên.
    Dangerous,
}

impl Risk {
    /// Có phải hỏi người dùng trước khi chạy không?
    #[must_use]
    pub const fn needs_confirmation(self) -> bool {
        matches!(self, Self::Confirm | Self::Dangerous)
    }

    /// Người dùng có được phép chọn "cho phép trong phiên" không?
    #[must_use]
    pub const fn allows_session_grant(self) -> bool {
        matches!(self, Self::Confirm)
    }

    /// Chuỗi hiển thị trong UI/Telegram.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Confirm => "confirm",
            Self::Dangerous => "dangerous",
        }
    }
}

/// Mô tả một tool để gửi cho model (schema sinh từ `schemars`, agents.md mục 7.1).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolSpec {
    /// Tên tool, duy nhất trong registry; MCP dùng tiền tố `mcp__<server>__<tool>`.
    pub name: String,
    /// Mô tả khi nào nên dùng tool — lấy từ doc comment của struct tham số.
    pub description: String,
    /// JSON Schema của tham số.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// Tạo một `ToolSpec`.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}
