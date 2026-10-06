//! Kiểu dữ liệu đi qua ranh giới store: record trả về cho lớp trên, và [`StoreError`].
//!
//! # Vì sao tách khỏi [`super`]
//!
//! Đây là **hợp đồng dữ liệu** của tầng lưu trữ: mọi thứ mà `bean-web` đọc để dựng
//! response, và mọi biến thể lỗi mà lớp trên phải phân nhánh. Chúng không thuộc về
//! bản cài đặt cụ thể nào — `MemoryStore` và `SqliteStore` dùng chung — nên tách riêng
//! giúp đổi một trong hai bản cài đặt không đụng tới kiểu.
//!
//! Quan trọng hơn: đây là **API công khai** (`pub`), được `lib.rs` re-export ra ngoài
//! crate. Giữ nó ở module riêng làm nơi kiểu được sửa đổi là duy nhất — trước khi tách,
//! sửa một struct ở đây còn phải lướt qua 4000 dòng SQL để tìm chỗ dùng nó.
use bean_types::{Message, Outbound, SessionId};
use serde::Serialize;
/// Thông tin một client MCP đã cấu quyền (M25).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpClientInfo {
    /// Tên client (không kèm tiền tố).
    pub name: String,
    /// Role mà client được cấp (tương ứng identity `mcp-client:<name>`).
    pub role: String,
    /// Thời điểm cấp token (RFC3339 UTC).
    pub created_at: String,
    /// Thời điểm hết hạn (RFC3339 UTC); rỗng ⇒ không hết hạn.
    pub expires_at: String,
}

/// Lỗi store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Phiên/tham chiếu không tồn tại.
    #[error("phiên không tồn tại: {0}")]
    NotFound(SessionId),
    /// Lỗi nội bộ (SQLite, worker dừng, dữ liệu hỏng…).
    #[error("lỗi nội bộ store: {0}")]
    Internal(String),
    /// Đã chạm hoặc vượt ngân sách token của ngày UTC hiện tại.
    #[error("đã dừng: ngân sách token/ngày đã đạt {used}/{limit} token")]
    BudgetExceeded {
        /// Tổng token đã ghi nhận trong ngày UTC.
        used: u64,
        /// Trần được cấu hình.
        limit: u64,
    },
}

/// Nguồn gốc của một kết quả `memory_search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemorySource {
    /// Ghi nhớ dài hạn (bảng `memories`).
    Memories,
    /// Lịch sử hội thoại (bảng `messages`).
    Message { session_id: i64 },
}

/// Một kết quả tìm kiếm bộ nhớ.
#[derive(Debug, Clone, Serialize)]
pub struct MemorySearchHit {
    /// Điểm liên quan đã **chuẩn hoá theo nguồn** (`1.0` = liên quan nhất của nguồn đó),
    /// nên so sánh được giữa `memories` và `messages`.
    pub score: f32,
    /// Nội dung (đã cắt theo [`MEMORY_HIT_PREVIEW_CHARS`] với kết quả từ `messages`).
    pub text: String,
    /// Nơi tìm thấy.
    pub source: MemorySource,
}

/// Một message đọc lên kèm số thứ tự trong phiên.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMessage {
    /// `messages.seq` — tăng dần trong phạm vi một phiên, bắt đầu từ 1.
    pub seq: u64,
    /// Nội dung.
    pub message: Message,
}

/// Thông tin ownership của một session, dùng để Router chống IDOR khi
/// `Incoming.session_id` được client cung cấp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Session cần kiểm tra.
    pub id: SessionId,
    /// Channel sở hữu session.
    pub channel: String,
    /// Chat ID sở hữu session.
    pub chat_id: String,
    /// User ID đã tạo/được phép dùng session.
    pub user_id: String,
    /// Session đã bị archive hay chưa.
    pub archived: bool,
}

/// Metadata session đầy đủ cho REST API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionSummary {
    /// ID session.
    pub id: SessionId,
    /// Channel sở hữu session.
    pub channel: String,
    /// Chat ID.
    pub chat_id: String,
    /// User ID sở hữu session.
    pub user_id: String,
    /// Tiêu đề hiển thị.
    pub title: String,
    /// Đã archive hay chưa.
    pub archived: bool,
    /// Thời điểm tạo RFC3339.
    pub created_at: String,
    /// Thời điểm cập nhật RFC3339.
    pub updated_at: String,
}

/// Message đầy đủ lấy theo ID cho REST API.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageRecord {
    /// ID message trong SQLite.
    pub id: i64,
    /// Session chứa message.
    pub session_id: SessionId,
    /// Thứ tự trong session.
    pub seq: u64,
    /// Nội dung message.
    pub message: Message,
    /// Thời điểm ghi.
    pub created_at: String,
}

/// Một ghi nhớ dài hạn cho REST API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryRecord {
    /// ID ghi nhớ.
    pub id: u64,
    /// Nội dung.
    pub text: String,
    /// Tag.
    pub tags: String,
    /// Thời điểm tạo.
    pub created_at: String,
}

/// Dữ liệu đầu vào khi tạo tác vụ định kỳ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewScheduledTask {
    /// Biểu thức cron, được hiểu theo `Config.agent.timezone` và lưu lịch UTC.
    pub cron: String,
    /// Prompt agent.
    pub prompt: String,
    /// Session sẽ chạy task; `None` chỉ dành cho dữ liệu migration cũ.
    pub session_id: Option<SessionId>,
    /// Channel nhận kết quả.
    pub channel: String,
    /// Chat ID nhận kết quả.
    pub chat_id: String,
    /// Danh sách tool được phép chạy tự động.
    pub allowed_tools: Vec<String>,
    /// Lần chạy kế tiếp UTC đã tính sẵn.
    pub next_run: String,
    /// Tác vụ có bật hay không.
    pub enabled: bool,
}

/// Một tác vụ định kỳ đã lưu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduledTask {
    /// ID tác vụ.
    pub id: u64,
    /// Biểu thức cron.
    pub cron: String,
    /// Prompt agent.
    pub prompt: String,
    /// Session sở hữu task; `None` với bản ghi cũ chưa migration.
    pub session_id: Option<SessionId>,
    /// Channel nhận kết quả.
    pub channel: String,
    /// Chat ID nhận kết quả.
    pub chat_id: String,
    /// Danh sách tool được phép chạy tự động.
    pub allowed_tools: Vec<String>,
    /// Lần chạy kế tiếp UTC.
    pub next_run: String,
    /// Tác vụ có bật hay không.
    pub enabled: bool,
    /// RFC3339 UTC lúc tạo.
    pub created_at: String,
    /// RFC3339 UTC của lần claim gần nhất.
    pub last_run_at: Option<String>,
    /// Trạng thái runtime gần nhất (pending/running/success/error/skipped).
    pub last_status: String,
}

/// Thông tin phiên đăng nhập sau khi token đã được xác thực.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSessionInfo {
    /// User ID của phiên.
    pub user_id: String,
    /// Thời điểm tạo.
    pub created_at: String,
    /// Thời điểm hết hạn.
    pub expires_at: String,
}

/// Một bản ghi outbox đã tới hạn gửi lại.
#[derive(Debug, Clone, PartialEq)]
pub struct OutboxEntry {
    /// Khoá chính.
    pub id: u64,
    /// Channel adapter cần gửi.
    pub channel: String,
    /// Chat ID đích.
    pub chat_id: String,
    /// Payload đã decode.
    pub payload: Outbound,
    /// Số lần thử đã thực hiện.
    pub attempts: u32,
    /// RFC3339 UTC của lần thử kế tiếp.
    pub next_attempt_at: String,
    /// Lỗi gần nhất, có thể rỗng.
    pub last_error: String,
    /// RFC3339 UTC lúc tạo.
    pub created_at: String,
}
