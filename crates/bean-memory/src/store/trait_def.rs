//! Trait [`Store`] — hợp đồng lưu trữ mà agent loop chỉ cần biết (agents.md mục 8).
//!
//! # Vì sao đây là file riêng
//!
//! Đây là **bề mặt ổn định nhất của cả tầng bộ nhớ**: `agent::run_turn` và mọi adapter
//! chỉ làm việc với `&dyn Store`, nên thêm một bản cài đặt mới (Postgres, Redis, hay
//! một fake gọn cho test) không đòi sửa một dòng nào ở nơi gọi.
//!
//! Giữ nó tách riêng còn giúp câu hỏi "cài đặt mới cần làm gì" trả lời được bằng
//! cách đọc **một** file, thay vì phải đọc cả `memory.rs` lẫn `sqlite/` để biết
//! `MemoryStore` và `SqliteStore` khác nhau ở đâu.
//!
//! # Vì sao trait này vẫn còn lớn
//!
//! 51 method trong một trait là **nợ kỹ thuật có thật**, không phải chuyện ngôn ngữ:
//! nó gộp sessions, messages, memories, tasks, usage, web_sessions, mcp_clients và
//! outbox vào một khái niệm. Tách theo nhóm nghiệp vụ (ISP) là việc riêng, cần đổi
//! chữ ký ở ~10 crate đang gọi — không làm trong cùng một đợt tách file như thế này.
//! Xem `docs/decisions.md` để biết kế hoạch.
use bean_llm::LlmProvider;
use bean_types::{Config, Message, Outbound, SessionId, Usage};

use super::types::{
    McpClientInfo, MemoryRecord, MemorySearchHit, MessageRecord, NewScheduledTask, OutboxEntry,
    ScheduledTask, SessionInfo, SessionSummary, StoreError, StoredMessage, WebSessionInfo,
};
#[async_trait::async_trait]
pub trait Store: Send + Sync {
    /// Ghi một message vào phiên (`seq` và message id do store cấp).
    async fn append(&self, session: SessionId, msg: Message) -> Result<i64, StoreError>;

    /// Lịch sử theo thứ tự cũ → mới.
    ///
    /// `before_seq = None` ⇒ lấy từ đầu; `limit = 0` ⇒ không giới hạn; ngược lại giữ
    /// **`limit` message cuối** (mới nhất) để luôn có phần hội thoại gần đây nhất.
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

    /// Lấy ownership của một session để Router kiểm tra `Incoming.session_id`.
    async fn session_info(&self, session: SessionId) -> Result<Option<SessionInfo>, StoreError>;

    /// Lấy id phiên đang hoạt động của `(channel, chat_id)`, tạo mới nếu chưa có.
    async fn ensure_session(
        &self,
        channel: &str,
        chat_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError>;

    /// Như [`Store::ensure_session`], đồng thời gắn ownership `user_id`.
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
    ///
    /// `expected_next_run` là điều kiện compare-and-swap; nhờ vậy hai tick
    /// đồng thời không thể claim cùng một occurrence.
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

    /// Liệt kê session của user, lọc theo q/title và archived.
    async fn list_sessions(
        &self,
        user_id: &str,
        query: Option<&str>,
        archived: Option<bool>,
        limit: usize,
    ) -> Result<Vec<SessionSummary>, StoreError>;

    /// Lấy message đầy đủ theo id.
    async fn message_by_id(&self, id: i64) -> Result<Option<MessageRecord>, StoreError>;

    /// Liệt kê message kèm id DB cho REST.
    async fn list_message_records(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, StoreError>;

    /// Liệt kê ghi nhớ dài hạn.
    async fn list_memories(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError>;

    /// Xoá một ghi nhớ dài hạn.
    async fn delete_memory(&self, id: u64) -> Result<bool, StoreError>;

    /// Cộng usage của một lượt gọi LLM vào ngày UTC.
    async fn add_usage(&self, day: &str, usage: Usage) -> Result<(), StoreError>;

    /// Đọc usage theo ngày UTC.
    async fn usage(&self, day: &str) -> Result<Usage, StoreError>;

    /// Cộng usage của **một role** trong ngày UTC (M21.7).
    ///
    /// Tách khỏi [`Self::add_usage`] (bảng tổng phục vụ `GET /api/status`) để mỗi role có hạn
    /// mức riêng: Developer chạy vòng lặp dài không ăn hết hạn mức khiến Monitor/Security-scan
    /// không chạy được job định kỳ.
    async fn add_usage_by_role(
        &self,
        day: &str,
        role: &str,
        usage: Usage,
    ) -> Result<(), StoreError>;

    /// Đọc usage của một role trong ngày UTC; chưa có ⇒ `Usage` rỗng.
    async fn usage_by_role(&self, day: &str, role: &str) -> Result<Usage, StoreError>;

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

    /// Cấp token cho một client MCP: chỉ lưu **hash** token (M25).
    ///
    /// `token_hash` là SHA-256 hex (64 ký tự) của token thô. Token thô chỉ in một lần
    /// lúc `bean auth mcp-token add` rồi không lưu ở đâu (mục 15.6).
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

    /// Đọc phần tóm tắt của phiên (`None` khi chưa có).
    async fn summary(&self, session: SessionId) -> Result<Option<String>, StoreError>;

    /// Lưu một ghi nhớ dài hạn; trả id vừa tạo.
    async fn memory_save(&self, text: &str, tags: &str) -> Result<u64, StoreError>;

    /// Tìm kiếm FTS5 trong cả `memories` và `messages`, sắp theo BM25.
    ///
    /// Query là đầu vào **không tin cậy** nên phải được làm sạch trước khi đưa vào
    /// `MATCH` (agents.md mục 8.4): từ khoá có ký tự điều khiển FTS không được làm
    /// hỏng truy vấn.
    async fn memory_search(&self, query: &str) -> Result<Vec<MemorySearchHit>, StoreError>;

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

    /// Danh sách message kèm `seq`, cũ → mới (dùng cho compaction).
    async fn list_messages(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, StoreError>;

    /// Xoá các message có `seq < before_seq` (compaction).
    async fn delete_before(&self, session: SessionId, before_seq: u64) -> Result<(), StoreError>;

    /// Nén lịch sử khi vượt 70% `agent.context_budget_tokens` (agents.md mục 8.3).
    ///
    /// Tóm tắt các message cũ vào `sessions.summary` rồi xoá chúng, **giữ nguyên**
    /// mọi cặp `assistant(tool_calls)`/`tool` nhờ chọn điểm cắt ở ranh giới an toàn.
    async fn compact(
        &self,
        session: SessionId,
        llm: &dyn LlmProvider,
        config: &Config,
    ) -> Result<(), StoreError>;
}
