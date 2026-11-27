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

/// Dữ liệu đầu vào khi đăng ký một subscription Web Push (trình duyệt gửi lên).
///
/// Tách khỏi [`PushSubscription`] (bản ghi đầy đủ) theo cùng mẫu
/// `NewScheduledTask`/`ScheduledTask`: bản ghi input không mang dữ liệu server
/// tự quản (số lần gửi lỗi) nên không thể nhầm lẫn ghi đè chúng từ ngoài.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPushSubscription {
    /// URL push service riêng cho trình duyệt này (do push service cấp, bí mật
    /// đối với bên thứ ba nhưng vẫn là dữ liệu không tin cậy từ client — không
    /// bao giờ đưa vào log nguyên văn).
    pub endpoint: String,
    /// Public key ECDH P-256 của trình duyệt (base64url, payload mã hoá).
    pub p256dh: String,
    /// Auth secret 16 byte (base64url) cho HKDF của ECE.
    pub auth: String,
    /// User-Agent của trình duyệt đăng ký — chỉ để hiển thị trong UI.
    pub user_agent: String,
    /// Thời điểm đăng ký RFC3339 UTC.
    pub created_at: String,
}

/// Một subscription Web Push đã lưu, kèm trạng thái runtime của server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushSubscription {
    /// URL push service (khoá chính).
    pub endpoint: String,
    /// Public key ECDH của trình duyệt.
    pub p256dh: String,
    /// Auth secret cho ECE.
    pub auth: String,
    /// User-Agent của trình duyệt đăng ký.
    pub user_agent: String,
    /// Thời điểm đăng ký RFC3339 UTC.
    pub created_at: String,
    /// Số lần gửi lỗi liên tiếp; gửi thành công reset về 0. Chỉ để chẩn đoán
    /// "điện thoại còn nhận thông báo không" — không tự xoá tại store.
    pub failure_count: u32,
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

// ---------------------------------------------------------------------------
// Bean World — bảng điều khiển công việc (projects → tasks → notes).
// Subagent (role) là "user" được giao việc; người thật và agent cùng dùng một
// nguồn sự thật duy nhất trong SQLite.
// ---------------------------------------------------------------------------

/// Yêu cầu tạo project mới (tool `project_create` / REST).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewWorkProject {
    /// Tên project, duy nhất, 1..=120 ký tự.
    pub name: String,
    /// Mô tả ngắn mục đích project; có thể rỗng.
    pub description: String,
}

/// Một project trong Bean World.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkProject {
    /// ID project.
    pub id: u64,
    /// Tên project, duy nhất.
    pub name: String,
    /// Mô tả ngắn.
    pub description: String,
    /// `active` hoặc `archived`.
    pub status: String,
    /// RFC3339 UTC lúc tạo.
    pub created_at: String,
    /// RFC3339 UTC lần sửa cuối.
    pub updated_at: String,
}

/// Bộ lọc khi liệt kê task; `None` = không lọc theo trường đó.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct WorkTaskFilter {
    /// Chỉ lấy task của project này.
    pub project_id: Option<u64>,
    /// Chỉ lấy task giao cho role này (khớp chính xác; rỗng = chưa giao).
    pub assignee: Option<String>,
    /// Chỉ lấy task ở trạng thái này (`open`/`in_progress`/`blocked`/`done`).
    pub status: Option<String>,
}

/// Yêu cầu tạo task mới (tool `task_add` / REST).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewWorkTask {
    /// Project chứa task.
    pub project_id: u64,
    /// Tiêu đề việc, 1..=200 ký tự.
    pub title: String,
    /// Thông tin liên quan ban đầu; có thể rỗng.
    pub notes: String,
    /// Role subagent được giao (`developer`, `qa`, `marketing`…); rỗng = chưa giao.
    pub assignee: String,
    /// `low` / `normal` / `high`.
    pub priority: String,
    /// Ai tạo: `user:<id>` hoặc `agent:<role>`.
    pub created_by: String,
}

/// Cập nhật một phần của task; `None` = giữ nguyên trường đó.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct WorkTaskUpdate {
    /// Đổi tiêu đề.
    pub title: Option<String>,
    /// Đổi thông tin liên quan (ghi đè).
    pub notes: Option<String>,
    /// Đổi trạng thái (`open`/`in_progress`/`blocked`/`done`).
    pub status: Option<String>,
    /// Đổi priority (`low`/`normal`/`high`).
    pub priority: Option<String>,
    /// Đổi người được giao; chuỗi rỗng = huỷ giao.
    pub assignee: Option<String>,
}

/// Một task trong Bean World.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkTask {
    /// ID task.
    pub id: u64,
    /// Project chứa task.
    pub project_id: u64,
    /// Tiêu đề việc.
    pub title: String,
    /// Thông tin liên quan.
    pub notes: String,
    /// `open`/`in_progress`/`blocked`/`done`.
    pub status: String,
    /// `low`/`normal`/`high`.
    pub priority: String,
    /// Role được giao; rỗng = chưa giao.
    pub assignee: String,
    /// Ai tạo: `user:<id>` hoặc `agent:<role>`.
    pub created_by: String,
    /// RFC3339 UTC lúc tạo.
    pub created_at: String,
    /// RFC3339 UTC lần sửa cuối.
    pub updated_at: String,
    /// RFC3339 UTC lúc xong; `None` khi chưa xong.
    pub done_at: Option<String>,
}

/// Yêu cầu thêm ghi chú trao đổi vào task (append-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewWorkNote {
    /// Task gắn ghi chú.
    pub task_id: u64,
    /// Ai viết: `user:<id>` hoặc `agent:<role>`.
    pub author: String,
    /// Nội dung ghi chú, 1..=4000 ký tự.
    pub text: String,
}

/// Một ghi chú trao đổi trên task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkNote {
    /// ID ghi chú.
    pub id: u64,
    /// Task chứa ghi chú.
    pub task_id: u64,
    /// Ai viết.
    pub author: String,
    /// Nội dung.
    pub text: String,
    /// RFC3339 UTC lúc viết.
    pub created_at: String,
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
