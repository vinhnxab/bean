//! Các định danh dùng xuyên suốt hệ thống.
//!
//! Dùng newtype thay vì `i64`/`String` trần để không lẫn `session_id` với `message_id`
//! hay `run_id` (agents.md mục 10, 11).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Khoá chính của bảng `sessions` (agents.md mục 8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(i64);

impl SessionId {
    /// Bọc một giá trị `rowid` của SQLite.
    #[must_use]
    pub const fn new(raw: i64) -> Self {
        Self(raw)
    }

    /// Giá trị thô (dùng khi truy vấn SQL).
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for SessionId {
    fn from(raw: i64) -> Self {
        Self(raw)
    }
}

/// Định danh một run (một lượt `run_turn`). Sinh ở Router, không phải ở kênh.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Tạo `RunId` từ chuỗi đã có (Router chịu trách nhiệm sinh chuỗi ngẫu nhiên khó đoán).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Chuỗi thô, dùng khi phát sự kiện ra WebSocket.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Định danh một yêu cầu xác nhận (agents.md mục 10).
///
/// Phải là chuỗi ngẫu nhiên khó đoán, gắn với đúng một run; ai phản hồi đầu tiên thì thắng.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConfirmId(String);

impl ConfirmId {
    /// Tạo `ConfirmId` từ chuỗi đã có.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Chuỗi thô.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ConfirmId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
