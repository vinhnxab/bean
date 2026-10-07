//! Trait Segregation - Interface Segregation Principle (ISP)
//!
//! # Mục tiêu
//!
//! Tách trait Store lớn (51 methods) thành các traits nhỏ hơn, each với single responsibility.
//! Client chỉ phụ thuộc vào traits họ actually use.
//!
//! # Cấu trúc
//!
//! ```text
//! Store (composite trait - backward compatibility)
//! ├── SessionStore   (14 methods) - session management
//! ├── MessageStore   (12 methods) - message operations
//! ├── MemoryStore    (6 methods)  - long-term memories
//! ├── TaskStore      (11 methods) - scheduled tasks
//! ├── UsageStore     (4 methods)  - token usage tracking
//! ├── OutboxStore    (5 methods)  - outbound messages
//! ├── WebSessionStore (5 methods) - web session management
//! └── McpClientStore (4 methods)  - MCP client management
//! ```

use std::sync::Arc;

use crate::store::types::StoreError;
use crate::store::types::SessionInfo;
use crate::store::types::SessionSummary;
use crate::store::types::ScheduledTask;
use crate::store::types::MemoryRecord;
use crate::store::types::MemorySearchHit;
use crate::store::types::MessageRecord;
use crate::store::types::NewScheduledTask;
use crate::store::types::OutboxEntry;
use crate::store::types::StoredMessage;
use crate::store::types::WebSessionInfo;
use crate::store::types::McpClientInfo;
use crate::store::types::Usage;
use bean_types::{Message, Outbound, SessionId};

// ============================================================================ //
// SESSION STORE - Single Responsibility: Session management
// ============================================================================ //

#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    /// Lấy ownership của một session để Router kiểm tra `Incoming.session_id`.
    async fn session_info(&self, session: SessionId) -> Result<Option<SessionInfo>, StoreError>;

    /// Lấy id phiên đang hoạt động của `(channel, chat_id)`, tạo mới nếu chưa có.
    async fn ensure_session(
        &self,
        channel: &str,
        chat_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError>;

    /// Như [`SessionStore::ensure_session`], đồng thời gắn ownership `user_id`.
    async fn ensure_session_for_user(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError>;

    /// Đánh dấu phiên đã lưu trữ (`/new`) — dữ liệu vẫn còn, chỉ không được chọn nữa.
    async fn archive_session(&self, session: SessionId) -> Result<(), StoreError>;

    /// Ghi đè phần tóm tắt của phiên (dùng cho compaction).
    async fn save_summary(&self, session: SessionId, summary: &str) -> Result<(), StoreError>;

    /// Tạo một session mới, không tái sử dụng session đang active.
    async fn create_session(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError>;

    /// Tải metadata session đầy đủ cho response web.
    async fn session_summary(
        &self,
        session: SessionId,
    ) -> Result<Option<SessionSummary>, StoreError>;

    /// Tìm session đang hoạt động theo channel/chat, không tạo session mới.
    async fn find_active_session(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Option<SessionId>, StoreError>;

    /// Liệt kê tác vụ định kỳ đã lưu.
    async fn list_tasks(&self) -> Result<Vec<ScheduledTask>, StoreError>;

    /// Liệt kê session của user, lọc theo q/title và archived.
    async fn list_sessions(
        &self,
        user_id: &str,
        query: Option<&str>,
        archived: Option<bool>,
        limit: usize,
    ) -> Result<Vec<SessionSummary>, StoreError>;

    /// Cập nhật metadata của session sau khi đã kiểm tra ownership.
    async fn update_session(
        &self,
        session: SessionId,
        user_id: &str,
        title: Option<&str>,
        archived: Option<bool>,
    ) -> Result<bool, StoreError>;

    /// Xoá session sau khi đã kiểm tra ownership.
    async fn delete_session(&self, session: SessionId, user_id: &str) -> Result<bool, StoreError>;

    /// Đọc phần tóm tắt của phiên (`None` khi chưa có).
    async fn summary(&self, session: SessionId) -> Result<Option<String>, StoreError>;
}

// ============================================================================ //
// MESSAGE STORE - Single Responsibility: Message operations
// ============================================================================ //

#[async_trait::async_trait]
pub trait MessageStore: Send + Sync {
    /// Ghi một message vào phiên (`seq` và message id do store cấp).
    async fn append(&self, session: SessionId, msg: Message) -> Result<i64, StoreError>;

    /// Lịch sử theo thứ tự cũ → mới.
    async fn history(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Message>, StoreError>;

    /// Số message hiện có của phiên.
    async fn count(&self, session: SessionId) -> Result<u64, StoreError>;

    /// Xoá **toàn bộ message** của phiên (giữ lại bản ghi phiên).
    async fn clear(&self, session: SessionId) -> Result<(), StoreError>;

    /// Lấy message đầy đủ theo id.
    async fn message_by_id(&self, id: i64) -> Result<Option<MessageRecord>, StoreError>;

    /// Liệt kê message kèm id DB cho REST.
    async fn list_message_records(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, StoreError>;

    /// Danh sách message kèm `seq`, cũ → mới (dùng cho compaction).
    async fn list_messages(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, StoreError>;

    /// Xoá các message có `seq < before_seq` (compaction).
    async fn delete_before(&self, session: SessionId, before_seq: u64) -> Result<(), StoreError>;
}

// ============================================================================ //
// MEMORY STORE - Single Responsibility: Long-term memory operations
// ============================================================================ //

#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    /// Liệt kê ghi nhớ dài hạn.
    async fn list_memories(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError>;

    /// Xoá một ghi nhớ dài hạn.
    async fn delete_memory(&self, id: u64) -> Result<bool, StoreError>;

    /// Lưu một ghi nhớ dài hạn; trả id vừa tạo.
    async fn memory_save(&self, text: &str, tags: &str) -> Result<u64, StoreError>;

    /// Tìm kiếm FTS5 trong cả `memories` và `messages`, sắp theo BM25.
    async fn memory_search(&self, query: &str) -> Result<Vec<MemorySearchHit>, StoreError>;
}

// ============================================================================ //
// TASK STORE - Single Responsibility: Scheduled task operations
// ============================================================================ //

#[async_trait::async_trait]
pub trait TaskStore: Send + Sync {
    /// Liệt kê task thuộc một session (tool model không được thấy task của session khác).
    async fn list_tasks_for_session(
        &self,
        session: SessionId,
    ) -> Result<Vec<ScheduledTask>, StoreError>;

    /// Tạo tác vụ định kỳ và trả bản ghi đã có ID.
    async fn create_task(&self, task: NewScheduledTask) -> Result<ScheduledTask, StoreError>;

    /// Bật/tắt tác vụ; trả bản ghi sau cập nhật hoặc `None` nếu không tồn tại.
    async fn set_task_enabled(
        &self,
        id: u64,
        enabled: bool,
    ) -> Result<Option<ScheduledTask>, StoreError>;

    /// Xoá task chỉ khi nó thuộc session; trả false nếu không tồn tại hoặc sai session.
    async fn delete_task_for_session(
        &self,
        id: u64,
        session: SessionId,
    ) -> Result<bool, StoreError>;

    /// Xoá tác vụ; trả false nếu không tồn tại.
    async fn delete_task(&self, id: u64) -> Result<bool, StoreError>;

    /// Các task đến hạn, cũ nhất trước.
    async fn due_tasks(&self, now: &str, limit: usize) -> Result<Vec<ScheduledTask>, StoreError>;

    /// Atomically claim một task và đẩy lịch sang occurrence kế tiếp.
    async fn claim_task(
        &self,
        id: u64,
        expected_next_run: &str,
        next_run: &str,
        last_run_at: &str,
        status: &str,
    ) -> Result<Option<ScheduledTask>, StoreError>;

    /// Cập nhật trạng thái runtime sau khi run kết thúc.
    async fn set_task_status(&self, id: u64, status: &str) -> Result<bool, StoreError>;
}

// ============================================================================ //
// USAGE STORE - Single Responsibility: Token usage tracking
// ============================================================================ //

#[async_trait::async_trait]
pub trait UsageStore: Send + Sync {
    /// Cộng usage của một lượt gọi LLM vào ngày UTC.
    async fn add_usage(&self, day: &str, usage: Usage) -> Result<(), StoreError>;

    /// Đọc usage theo ngày UTC.
    async fn usage(&self, day: &str) -> Result<Usage, StoreError>;

    /// Cộng usage của **một role** trong ngày UTC (M21.7).
    async fn add_usage_by_role(
        &self,
        day: &str,
        role: &str,
        usage: Usage,
    ) -> Result<(), StoreError>;

    /// Đọc usage của một role trong ngày UTC; chưa có ⇒ `Usage` rỗng.
    async fn usage_by_role(&self, day: &str, role: &str) -> Result<Usage, StoreError>;
}

// ============================================================================ //
// OUTBOX STORE - Single Responsibility: Outbound message queue
// ============================================================================ //

#[async_trait::async_trait]
pub trait OutboxStore: Send + Sync {
    /// Lưu outbound để gửi lại; `next_attempt_at` là RFC3339 UTC cố định.
    async fn enqueue_outbound(
        &self,
        channel: &str,
        chat_id: &str,
        payload: &Outbound,
        next_attempt_at: &str,
    ) -> Result<u64, StoreError>;

    /// Các bản ghi outbox đến hạn, cũ nhất trước, tối đa `limit` bản ghi.
    async fn due_outbox(&self, now: &str, limit: usize) -> Result<Vec<OutboxEntry>, StoreError>;

    /// Xoá bản ghi sau khi channel gửi thành công.
    async fn complete_outbox(&self, id: u64) -> Result<(), StoreError>;

    /// Cập nhật lần thử thất bại và lịch retry mới.
    async fn retry_outbox(
        &self,
        id: u64,
        next_attempt_at: &str,
        last_error: &str,
    ) -> Result<(), StoreError>;
}

// ============================================================================ //
// WEB SESSION STORE - Single Responsibility: Web session management
// ============================================================================ //

#[async_trait::async_trait]
pub trait WebSessionStore: Send + Sync {
    /// Tạo bản ghi phiên đăng nhập chỉ lưu hash token.
    async fn create_web_session(
        &self,
        token_hash: Vec<u8>,
        user_id: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), StoreError>;

    /// Kiểm tra hash token còn hợp lệ theo thời điểm UTC.
    async fn get_web_session(
        &self,
        token_hash: &[u8],
        now: &str,
    ) -> Result<Option<WebSessionInfo>, StoreError>;

    /// Cập nhật last_seen của phiên đăng nhập.
    async fn touch_web_session(&self, token_hash: &[u8], now: &str) -> Result<bool, StoreError>;

    /// Xoá một phiên đăng nhập.
    async fn delete_web_session(&self, token_hash: &[u8]) -> Result<bool, StoreError>;

    /// Xoá toàn bộ phiên đăng nhập (khi đổi mật khẩu).
    async fn delete_all_web_sessions(&self) -> Result<(), StoreError>;
}

// ============================================================================ //
// MCP CLIENT STORE - Single Responsibility: MCP client management
// ============================================================================ //

#[async_trait::async_trait]
pub trait McpClientStore: Send + Sync {
    /// Cấp token cho một client MCP: chỉ lưu **hash** token (M25).
    async fn create_mcp_client(
        &self,
        token_hash: &str,
        name: &str,
        role: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), StoreError>;

    /// Tra client MCP theo hash token; `None` khi token sai hoặc đã hết hạn.
    async fn get_mcp_client(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<Option<McpClientInfo>, StoreError>;

    /// Liệt kê client MCP đã cấp token (theo thời điểm cấp tăng dần).
    async fn list_mcp_clients(&self) -> Result<Vec<McpClientInfo>, StoreError>;

    /// Thu hồi token của một client theo tên; `false` khi không có client đó.
    async fn delete_mcp_client(&self, name: &str) -> Result<bool, StoreError>;
}
