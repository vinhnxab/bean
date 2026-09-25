//! Interface lưu trữ lịch sử hội thoại (agents.md mục 8).
//!
//! * **M3**: bản in-memory [`MemoryStore`] — nhanh, dùng cho test vòng lặp và demo.
//! * **M5**: bản SQLite + FTS5 [`SqliteStore`] — bền vững qua khởi động lại, tìm kiếm
//!   BM25 (`memory_search`), compaction ở **ranh giới an toàn** (`crate::safe_cut`).
//!
//! Cả hai cùng cài đặt **một** trait [`Store`], nên `agent::run_turn` không cần biết
//! lịch sử đang nằm ở đâu.
//!
//! Bất biến (agents.md mục 8.3, 22.1): không thao tác nào được tách một cặp
//! `assistant(tool_calls)` khỏi các `tool` result của nó — API sẽ trả 400.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use beanagent_llm::{ChatRequest, LlmProvider};
use beanagent_types::{Config, Message, Outbound, Role, SessionId, Usage};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use tokio::sync::{RwLock, oneshot};

use crate::safe_cut::{check_no_orphan_result, find_compaction_start};

/// Số message giữ lại (tối thiểu) sau một lần compaction.
const COMPACT_KEEP_RECENT: usize = 20;
/// Ngưỡng kích hoạt compaction: dùng > 70% ngân sách context (agents.md mục 8.3).
const COMPACT_TRIGGER_PERCENT: u64 = 70;
/// Ước lượng token khi provider không có API đếm: `chars / 4` (agents.md mục 8.2).
const CHARS_PER_TOKEN: u64 = 4;
/// Trần ký tự đưa vào prompt tóm tắt (chặn nổ context khi lịch sử quá dài).
const MAX_SUMMARY_INPUT_CHARS: usize = 48_000;
/// Số kết quả tối đa của một lần `memory_search`.
const MEMORY_SEARCH_LIMIT: usize = 20;
/// Độ dài preview của message trả về từ `memory_search`.
const MEMORY_HIT_PREVIEW_CHARS: usize = 500;

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

/// Giao diện lưu trữ lịch sử hội thoại, bộ nhớ dài hạn và compaction.
///
/// Các hàm trả `Result<_, StoreError>` chứ **không** `panic` (agents.md mục 0.8).
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

/// Kiểm tra ngân sách trước một lượt gọi LLM mới.
///
/// `Usage` là số token thực trả về bởi provider. Khi đã chạm trần, caller phải dừng và
/// báo người dùng; lượt gọi mới không được gửi đi. Ngày được truyền rõ ràng để test và
/// scheduler có thể kiểm soát mốc UTC.
pub async fn ensure_daily_budget(
    store: &dyn Store,
    day: &str,
    limit: u64,
) -> Result<(), StoreError> {
    let usage = store.usage(day).await?;
    let used = u64::from(usage.total());
    if used >= limit {
        return Err(StoreError::BudgetExceeded { used, limit });
    }
    Ok(())
}

/// Ghi usage và trả tổng mới của ngày.
///
/// Việc đọc lại sau `add_usage` giữ API store đơn giản cho cả SQLite và MemoryStore;
/// lớp gọi LLM chỉ cần một cổng cập nhật usage duy nhất.
pub async fn record_usage(store: &dyn Store, day: &str, usage: Usage) -> Result<Usage, StoreError> {
    store.add_usage(day, usage).await?;
    store.usage(day).await
}

// ---------------------------------------------------------------------------
// Bản in-memory (M3)
// ---------------------------------------------------------------------------

/// Một phiên trong bản in-memory.
#[derive(Debug, Clone)]
struct MemorySession {
    id: SessionId,
    channel: String,
    chat_id: String,
    user_id: String,
    title: String,
    archived: bool,
    summary: String,
    messages: Vec<Message>,
    message_ids: Vec<i64>,
}

/// Một ghi nhớ dài hạn trong bản in-memory.
#[derive(Debug, Clone)]
struct MemoryEntry {
    id: u64,
    text: String,

    tags: String,
    created_at: String,
}

/// Store **in-memory**: không I/O, không bền vững — dùng cho test vòng lặp agent
/// (`beanagent-core/tests/agent_loop.rs`) và demo không cần đụng đĩa.
/// Một bản ghi outbox trong bản in-memory.
#[derive(Debug, Clone)]

struct MemoryOutboxEntry {
    entry: OutboxEntry,
}

#[derive(Debug)]
pub struct MemoryStore {
    sessions: RwLock<Vec<MemorySession>>,
    memories: RwLock<Vec<MemoryEntry>>,
    tasks: RwLock<Vec<ScheduledTask>>,
    outbox: RwLock<Vec<MemoryOutboxEntry>>,
    usage: RwLock<BTreeMap<String, Usage>>,
    web_sessions: RwLock<Vec<(Vec<u8>, WebSessionInfo)>>,
    next_session_id: RwLock<i64>,
    next_memory_id: RwLock<u64>,
    next_task_id: RwLock<u64>,
    next_message_id: RwLock<u64>,
    next_outbox_id: RwLock<u64>,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self {
            sessions: RwLock::new(Vec::new()),
            memories: RwLock::new(Vec::new()),
            tasks: RwLock::new(Vec::new()),
            outbox: RwLock::new(Vec::new()),
            usage: RwLock::new(BTreeMap::new()),
            web_sessions: RwLock::new(Vec::new()),
            next_session_id: RwLock::new(1),
            next_memory_id: RwLock::new(1),
            next_task_id: RwLock::new(1),
            next_message_id: RwLock::new(1),
            next_outbox_id: RwLock::new(1),
        }
    }
}

impl MemoryStore {
    /// Tạo store in-memory rỗng.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    async fn append(&self, session: SessionId, msg: Message) -> Result<i64, StoreError> {
        let message_id = {
            let mut next = self.next_message_id.write().await;
            let id = *next;
            *next = next.saturating_add(1);
            i64::try_from(id)
                .map_err(|_| StoreError::Internal("message id vượt giới hạn".into()))?
        };
        let mut sessions = self.sessions.write().await;
        match sessions.iter_mut().find(|s| s.id == session) {
            Some(s) => {
                if s.title.is_empty()
                    && let Some(title) = title_from(&msg)
                {
                    s.title = title;
                }
                s.messages.push(msg);
                s.message_ids.push(message_id);
            }
            None => {
                // Tự tạo phiên: test M3 ghi thẳng vào `SessionId::new(1)` không qua
                // `ensure_session`, nên `append` phải tha thứ thay vì trả lỗi.
                let title = title_from(&msg).unwrap_or_default();
                sessions.push(MemorySession {
                    id: session,
                    channel: String::new(),
                    chat_id: String::new(),
                    user_id: String::new(),
                    title,
                    archived: false,
                    summary: String::new(),
                    messages: vec![msg],
                    message_ids: vec![message_id],
                });
            }
        }
        Ok(message_id)
    }

    async fn history(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Message>, StoreError> {
        let sessions = self.sessions.read().await;
        let Some(s) = sessions.iter().find(|s| s.id == session) else {
            return Ok(Vec::new());
        };
        let upper = seq_to_index(before_seq, s.messages.len());
        let start = slice_start(upper, limit);
        Ok(s.messages[start..upper].to_vec())
    }

    async fn count(&self, session: SessionId) -> Result<u64, StoreError> {
        let sessions = self.sessions.read().await;
        let n = sessions
            .iter()
            .find(|s| s.id == session)
            .map_or(0, |s| s.messages.len());
        Ok(n as u64)
    }

    async fn clear(&self, session: SessionId) -> Result<(), StoreError> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session) {
            s.messages.clear();
            s.message_ids.clear();
        }
        Ok(())
    }

    async fn session_info(&self, session: SessionId) -> Result<Option<SessionInfo>, StoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions
            .iter()
            .find(|item| item.id == session)
            .map(|item| SessionInfo {
                id: item.id,
                channel: item.channel.clone(),
                chat_id: item.chat_id.clone(),
                user_id: item.user_id.clone(),
                archived: item.archived,
            }))
    }

    async fn ensure_session(
        &self,
        channel: &str,
        chat_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions
            .iter()
            .rev()
            .find(|s| !s.archived && s.channel == channel && s.chat_id == chat_id)
        {
            return Ok(s.id);
        }
        let mut next = self.next_session_id.write().await;
        let id = SessionId::new(*next);
        *next += 1;
        sessions.push(MemorySession {
            id,
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            user_id: String::new(),
            title: title.to_string(),
            archived: false,
            summary: String::new(),
            messages: Vec::new(),
            message_ids: Vec::new(),
        });
        Ok(id)
    }

    async fn ensure_session_for_user(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.iter_mut().rev().find(|s| {
            !s.archived
                && s.channel == channel
                && s.chat_id == chat_id
                && (s.user_id.is_empty() || s.user_id == user_id)
        }) {
            s.user_id = user_id.to_string();
            return Ok(s.id);
        }
        let mut next = self.next_session_id.write().await;
        let id = SessionId::new(*next);
        *next += 1;
        sessions.push(MemorySession {
            id,
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            user_id: user_id.to_string(),
            title: title.to_string(),
            archived: false,
            summary: String::new(),
            messages: Vec::new(),
            message_ids: Vec::new(),
        });
        Ok(id)
    }

    async fn archive_session(&self, session: SessionId) -> Result<(), StoreError> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session) {
            s.archived = true;
        }
        Ok(())
    }

    async fn save_summary(&self, session: SessionId, summary: &str) -> Result<(), StoreError> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session) {
            s.summary = summary.to_string();
        }
        Ok(())
    }

    async fn summary(&self, session: SessionId) -> Result<Option<String>, StoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions
            .iter()
            .find(|s| s.id == session)
            .map(|s| s.summary.clone())
            .filter(|text| !text.is_empty()))
    }

    async fn create_session(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError> {
        let mut sessions = self.sessions.write().await;
        let mut next = self.next_session_id.write().await;
        let id = SessionId::new(*next);
        *next += 1;
        sessions.push(MemorySession {
            id,
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            user_id: user_id.to_string(),
            title: title.to_string(),
            archived: false,
            summary: String::new(),
            messages: Vec::new(),
            message_ids: Vec::new(),
        });
        Ok(id)
    }

    async fn session_summary(
        &self,
        session: SessionId,
    ) -> Result<Option<SessionSummary>, StoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions
            .iter()
            .find(|item| item.id == session)
            .map(|item| SessionSummary {
                id: item.id,
                channel: item.channel.clone(),
                chat_id: item.chat_id.clone(),
                user_id: item.user_id.clone(),
                title: item.title.clone(),
                archived: item.archived,
                created_at: String::new(),
                updated_at: String::new(),
            }))
    }

    async fn find_active_session(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Option<SessionId>, StoreError> {
        let sessions = self.sessions.read().await;
        Ok(sessions
            .iter()
            .rev()
            .find(|item| !item.archived && item.channel == channel && item.chat_id == chat_id)
            .map(|item| item.id))
    }

    async fn update_session(
        &self,
        session: SessionId,
        user_id: &str,
        title: Option<&str>,
        archived: Option<bool>,
    ) -> Result<bool, StoreError> {
        let mut sessions = self.sessions.write().await;
        let Some(item) = sessions
            .iter_mut()
            .find(|s| s.id == session && s.user_id == user_id)
        else {
            return Ok(false);
        };
        if let Some(value) = title {
            item.title = value.to_string();
        }
        if let Some(value) = archived {
            item.archived = value;
        }
        Ok(true)
    }

    async fn delete_session(&self, session: SessionId, user_id: &str) -> Result<bool, StoreError> {
        let mut sessions = self.sessions.write().await;
        let before = sessions.len();
        sessions.retain(|s| !(s.id == session && s.user_id == user_id));
        Ok(sessions.len() != before)
    }

    async fn list_sessions(
        &self,
        user_id: &str,
        query: Option<&str>,
        archived: Option<bool>,
        limit: usize,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let needle = query.unwrap_or_default().trim().to_lowercase();
        let sessions = self.sessions.read().await;
        let mut result: Vec<_> = sessions
            .iter()
            .filter(|s| s.user_id == user_id)
            .filter(|s| archived.is_none_or(|value| s.archived == value))
            .filter(|s| needle.is_empty() || s.title.to_lowercase().contains(&needle))
            .map(|s| SessionSummary {
                id: s.id,
                channel: s.channel.clone(),
                chat_id: s.chat_id.clone(),
                user_id: s.user_id.clone(),
                title: s.title.clone(),
                archived: s.archived,
                created_at: String::new(),
                updated_at: String::new(),
            })
            .collect();
        if limit > 0 {
            result.truncate(limit);
        }
        Ok(result)
    }

    async fn message_by_id(&self, id: i64) -> Result<Option<MessageRecord>, StoreError> {
        let sessions = self.sessions.read().await;
        for session in sessions.iter() {
            for (index, message) in session.messages.iter().enumerate() {
                if session.message_ids.get(index).copied() == Some(id) {
                    return Ok(Some(MessageRecord {
                        id,
                        session_id: session.id,
                        seq: index as u64 + 1,
                        message: message.clone(),
                        created_at: String::new(),
                    }));
                }
            }
        }
        Ok(None)
    }

    async fn list_message_records(
        &self,
        session_id: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, StoreError> {
        let sessions = self.sessions.read().await;
        let Some(session) = sessions.iter().find(|item| item.id == session_id) else {
            return Ok(Vec::new());
        };
        let upper = seq_to_index(before_seq, session.messages.len());
        let start = slice_start(upper, limit);
        Ok(session
            .messages
            .iter()
            .enumerate()
            .skip(start)
            .take(upper.saturating_sub(start))
            .map(|(index, message)| MessageRecord {
                id: session.message_ids.get(index).copied().unwrap_or_default(),
                session_id,
                seq: index as u64 + 1,
                message: message.clone(),
                created_at: String::new(),
            })
            .collect())
    }

    async fn list_memories(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let needle = query.unwrap_or_default().trim().to_lowercase();
        let memories = self.memories.read().await;
        let mut result: Vec<_> = memories
            .iter()
            .filter(|item| {
                needle.is_empty()
                    || item.text.to_lowercase().contains(&needle)
                    || item.tags.to_lowercase().contains(&needle)
            })
            .map(|item| MemoryRecord {
                id: item.id,
                text: item.text.clone(),
                tags: item.tags.clone(),
                created_at: item.created_at.clone(),
            })
            .collect();
        if limit > 0 {
            result.truncate(limit);
        }
        Ok(result)
    }

    async fn delete_memory(&self, id: u64) -> Result<bool, StoreError> {
        let mut memories = self.memories.write().await;
        let index = memories
            .iter()
            .position(|item| item.id == id)
            .ok_or(StoreError::NotFound(SessionId::new(0)))?;
        memories.remove(index);
        Ok(true)
    }

    async fn list_tasks(&self) -> Result<Vec<ScheduledTask>, StoreError> {
        Ok(self.tasks.read().await.clone())
    }

    async fn list_tasks_for_session(
        &self,
        session: SessionId,
    ) -> Result<Vec<ScheduledTask>, StoreError> {
        Ok(self
            .tasks
            .read()
            .await
            .iter()
            .filter(|task| task.session_id == Some(session))
            .cloned()
            .collect())
    }

    async fn create_task(&self, task: NewScheduledTask) -> Result<ScheduledTask, StoreError> {
        let id = {
            let mut next = self.next_task_id.write().await;
            let value = *next;
            *next = next.saturating_add(1);
            value
        };
        let created_at = now_rfc3339();
        let record = ScheduledTask {
            id,
            cron: task.cron,
            prompt: task.prompt,
            session_id: task.session_id,
            channel: task.channel,
            chat_id: task.chat_id,
            allowed_tools: task.allowed_tools,
            next_run: task.next_run,
            enabled: task.enabled,
            created_at: created_at.clone(),
            last_run_at: None,
            last_status: if task.enabled { "pending" } else { "disabled" }.into(),
        };
        self.tasks.write().await.push(record.clone());
        Ok(record)
    }

    async fn set_task_enabled(
        &self,
        id: u64,
        enabled: bool,
    ) -> Result<Option<ScheduledTask>, StoreError> {
        let mut tasks = self.tasks.write().await;
        let Some(task) = tasks.iter_mut().find(|task| task.id == id) else {
            return Ok(None);
        };
        task.enabled = enabled;
        if !enabled {
            task.last_status = "disabled".into();
        } else if task.last_status == "disabled" {
            task.last_status = "pending".into();
        }
        Ok(Some(task.clone()))
    }

    async fn delete_task_for_session(
        &self,
        id: u64,
        session: SessionId,
    ) -> Result<bool, StoreError> {
        let mut tasks = self.tasks.write().await;
        let before = tasks.len();
        tasks.retain(|task| task.id != id || task.session_id != Some(session));
        Ok(tasks.len() != before)
    }

    async fn delete_task(&self, id: u64) -> Result<bool, StoreError> {
        let mut tasks = self.tasks.write().await;
        let before = tasks.len();
        tasks.retain(|task| task.id != id);
        Ok(tasks.len() != before)
    }

    async fn due_tasks(&self, now: &str, limit: usize) -> Result<Vec<ScheduledTask>, StoreError> {
        let tasks = self.tasks.read().await;
        let mut result: Vec<_> = tasks
            .iter()
            .filter(|task| {
                task.enabled && task.last_status != "running" && task.next_run.as_str() <= now
            })
            .cloned()
            .collect();
        result.sort_by(|left, right| {
            left.next_run
                .cmp(&right.next_run)
                .then_with(|| left.id.cmp(&right.id))
        });
        if limit > 0 {
            result.truncate(limit);
        }
        Ok(result)
    }

    async fn claim_task(
        &self,
        id: u64,
        expected_next_run: &str,
        next_run: &str,
        last_run_at: &str,
        status: &str,
    ) -> Result<Option<ScheduledTask>, StoreError> {
        let mut tasks = self.tasks.write().await;
        let Some(task) = tasks
            .iter_mut()
            .find(|task| task.id == id && task.enabled && task.next_run == expected_next_run)
        else {
            return Ok(None);
        };
        task.next_run = next_run.to_string();
        task.last_run_at = Some(last_run_at.to_string());
        task.last_status = status.to_string();
        Ok(Some(task.clone()))
    }

    async fn set_task_status(&self, id: u64, status: &str) -> Result<bool, StoreError> {
        let mut tasks = self.tasks.write().await;
        let Some(task) = tasks.iter_mut().find(|task| task.id == id) else {
            return Ok(false);
        };
        task.last_status = status.to_string();
        Ok(true)
    }

    async fn add_usage(&self, day: &str, usage: Usage) -> Result<(), StoreError> {
        let mut values = self.usage.write().await;
        let current = values.entry(day.to_string()).or_default();
        current.input_tokens = current.input_tokens.saturating_add(usage.input_tokens);
        current.output_tokens = current.output_tokens.saturating_add(usage.output_tokens);
        Ok(())
    }

    async fn usage(&self, day: &str) -> Result<Usage, StoreError> {
        Ok(self
            .usage
            .read()
            .await
            .get(day)
            .copied()
            .unwrap_or_default())
    }

    async fn create_web_session(
        &self,
        token_hash: Vec<u8>,
        user_id: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), StoreError> {
        self.web_sessions.write().await.push((
            token_hash,
            WebSessionInfo {
                user_id: user_id.to_string(),
                created_at: created_at.to_string(),
                expires_at: expires_at.to_string(),
            },
        ));
        Ok(())
    }

    async fn get_web_session(
        &self,
        token_hash: &[u8],
        now: &str,
    ) -> Result<Option<WebSessionInfo>, StoreError> {
        Ok(self
            .web_sessions
            .read()
            .await
            .iter()
            .find(|(hash, info)| hash == token_hash && info.expires_at.as_str() > now)
            .map(|(_, info)| info.clone()))
    }

    async fn touch_web_session(&self, token_hash: &[u8], _now: &str) -> Result<bool, StoreError> {
        let sessions = self.web_sessions.read().await;
        Ok(sessions.iter().any(|(hash, _)| hash == token_hash))
    }

    async fn delete_web_session(&self, token_hash: &[u8]) -> Result<bool, StoreError> {
        let mut sessions = self.web_sessions.write().await;
        let before = sessions.len();
        sessions.retain(|(hash, _)| hash != token_hash);
        Ok(sessions.len() != before)
    }

    async fn delete_all_web_sessions(&self) -> Result<(), StoreError> {
        self.web_sessions.write().await.clear();
        Ok(())
    }

    async fn memory_save(&self, text: &str, tags: &str) -> Result<u64, StoreError> {
        let mut memories = self.memories.write().await;
        let mut next = self.next_memory_id.write().await;
        let id = *next;
        *next += 1;
        memories.push(MemoryEntry {
            id,
            text: text.to_string(),
            tags: tags.to_string(),
            created_at: now_rfc3339(),
        });
        Ok(id)
    }

    async fn memory_search(&self, query: &str) -> Result<Vec<MemorySearchHit>, StoreError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let memories = self.memories.read().await;
        Ok(memories
            .iter()
            .filter(|e| {
                e.text.to_lowercase().contains(&needle) || e.tags.to_lowercase().contains(&needle)
            })
            .take(MEMORY_SEARCH_LIMIT)
            .map(|e| MemorySearchHit {
                score: 1.0,
                text: truncate_chars(&e.text, MEMORY_HIT_PREVIEW_CHARS),
                source: MemorySource::Memories,
            })
            .collect())
    }

    async fn enqueue_outbound(
        &self,
        channel: &str,
        chat_id: &str,
        payload: &Outbound,
        next_attempt_at: &str,
    ) -> Result<u64, StoreError> {
        let id = {
            let mut next = self.next_outbox_id.write().await;
            let id = *next;
            *next = next.saturating_add(1);
            id
        };
        self.outbox.write().await.push(MemoryOutboxEntry {
            entry: OutboxEntry {
                id,
                channel: channel.to_string(),
                chat_id: chat_id.to_string(),
                payload: payload.clone(),
                attempts: 0,
                next_attempt_at: next_attempt_at.to_string(),
                last_error: String::new(),
                created_at: now_rfc3339(),
            },
        });
        Ok(id)
    }

    async fn due_outbox(&self, now: &str, limit: usize) -> Result<Vec<OutboxEntry>, StoreError> {
        let outbox = self.outbox.read().await;
        let mut entries: Vec<_> = outbox
            .iter()
            .map(|item| item.entry.clone())
            .filter(|entry| entry.next_attempt_at.as_str() <= now)
            .collect();
        entries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        if limit > 0 {
            entries.truncate(limit);
        }
        Ok(entries)
    }

    async fn complete_outbox(&self, id: u64) -> Result<(), StoreError> {
        self.outbox.write().await.retain(|item| item.entry.id != id);
        Ok(())
    }

    async fn retry_outbox(
        &self,
        id: u64,
        next_attempt_at: &str,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let mut outbox = self.outbox.write().await;
        if let Some(item) = outbox.iter_mut().find(|item| item.entry.id == id) {
            item.entry.attempts = item.entry.attempts.saturating_add(1);
            item.entry.next_attempt_at = next_attempt_at.to_string();
            item.entry.last_error = truncate_chars(last_error, 1_000);
        }
        Ok(())
    }

    async fn list_messages(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, StoreError> {
        let sessions = self.sessions.read().await;
        let Some(s) = sessions.iter().find(|s| s.id == session) else {
            return Ok(Vec::new());
        };
        let upper = seq_to_index(before_seq, s.messages.len());
        let start = slice_start(upper, limit);
        Ok(s.messages[start..upper]
            .iter()
            .enumerate()
            .map(|(offset, message)| StoredMessage {
                seq: (start + offset) as u64 + 1,
                message: message.clone(),
            })
            .collect())
    }

    async fn delete_before(&self, session: SessionId, before_seq: u64) -> Result<(), StoreError> {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session) {
            let keep_from = (before_seq.saturating_sub(1) as usize).min(s.messages.len());
            s.messages.drain(0..keep_from);
            let remove_ids = keep_from.min(s.message_ids.len());
            s.message_ids.drain(0..remove_ids);
        }
        Ok(())
    }

    async fn compact(
        &self,
        session: SessionId,
        llm: &dyn LlmProvider,
        config: &Config,
    ) -> Result<(), StoreError> {
        let store: &dyn Store = self;
        compact_via(store, session, llm, config).await
    }
}

// ---------------------------------------------------------------------------
// Helper dùng chung cho cả hai bản cài đặt
// ---------------------------------------------------------------------------

/// Tiêu đề hiển thị trong UI, lấy từ **dòng đầu không rỗng** của tin đầu tiên do người
/// dùng gửi (agents.md mục 8.1). Trả `None` nếu message không phải của người dùng.
fn title_from(msg: &Message) -> Option<String> {
    if msg.role != Role::User {
        return None;
    }
    let text = msg.text.as_deref().unwrap_or_default();
    let first_line = text.lines().find(|line| !line.trim().is_empty())?;
    Some(truncate_chars(first_line.trim(), 60))
}

/// Cắt chuỗi tại **ranh giới ký tự** (agents.md mục 22.9) — không bao giờ panic với
/// tiếng Việt hay emoji.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// `before_seq` (1-based) ⇒ số phần tử cần giữ ở đầu danh sách.
fn seq_to_index(before_seq: Option<u64>, len: usize) -> usize {
    match before_seq {
        Some(seq) => (seq.saturating_sub(1) as usize).min(len),
        None => len,
    }
}

/// Vị trí bắt đầu của cửa sổ `limit` phần tử cuối (`limit = 0` ⇒ từ đầu).
fn slice_start(upper: usize, limit: usize) -> usize {
    if limit == 0 {
        0
    } else {
        upper.saturating_sub(limit)
    }
}

/// `LIMIT` cho SQLite: `0` trong API nghĩa là "không giới hạn" ⇒ `-1`.
fn sql_limit(limit: usize) -> i64 {
    if limit == 0 { -1 } else { limit as i64 }
}

/// Ước lượng token theo `chars / 4` khi provider không có API đếm (agents.md mục 8.2).
fn estimate_tokens(messages: &[Message]) -> u64 {
    let chars: u64 = messages
        .iter()
        .map(|msg| msg.text_for_search().chars().count() as u64)
        .sum();
    chars / CHARS_PER_TOKEN
}

/// Chuyển lịch sử thành transcript cho prompt tóm tắt (kèm hành động tool).
fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for msg in messages {
        out.push_str(msg.role.as_str());
        out.push_str(": ");
        if let Some(text) = &msg.text {
            out.push_str(text);
        }
        for call in &msg.tool_calls {
            out.push_str(&format!("\n  [gọi {}] {}", call.name, call.args));
        }
        if msg.is_error {
            out.push_str("\n  (lỗi)");
        }
        out.push('\n');
    }
    out
}

/// System prompt cho lượt "reflection" chỉ để nén lịch sử (agents.md mục 8.3).
const SUMMARY_SYSTEM_PROMPT: &str = "You compress conversation history for a personal AI agent. \
Write a dense factual summary in the same language as the conversation. Keep: the current goal, \
decisions already made, important file paths and commands, and unfinished work. Never invent \
facts that are not in the transcript. Output only the summary, no preamble.";

/// Gọi LLM một lượt để tóm tắt phần lịch sử cũ.
async fn summarize(
    store: &dyn Store,
    llm: &dyn LlmProvider,
    config: &Config,
    previous: Option<&str>,
    old: &[Message],
) -> Result<String, StoreError> {
    let transcript = truncate_chars(&render_transcript(old), MAX_SUMMARY_INPUT_CHARS);
    let mut user = String::new();
    if let Some(prev) = previous {
        user.push_str("Tóm tắt đã có của phần hội thoại trước đó:\n");
        user.push_str(prev);
        user.push_str("\n\n");
    }
    user.push_str("Hội thoại cần nén (cũ → mới):\n");
    user.push_str(&transcript);
    user.push_str(
        "\nHãy viết bản tóm tắt mới gộp cả phần đã có ở trên và phần vừa nêu. Giữ: mục tiêu đang làm, \
         quyết định đã chốt, file/đường dẫn quan trọng, việc còn dang dở.",
    );

    let messages = [Message::user(user)];
    let request = ChatRequest {
        system: SUMMARY_SYSTEM_PROMPT,
        messages: &messages,
        tools: &[],
        max_tokens: config.llm.max_tokens.min(2048),
    };
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    ensure_daily_budget(store, &day, config.security.daily_token_budget).await?;
    let response = llm
        .chat_with_model(request, &config.llm.model)
        .await
        .map_err(|err| StoreError::Internal(format!("gọi LLM để tóm tắt thất bại: {err}")))?;
    let usage = record_usage(store, &day, response.usage).await?;
    let used = u64::from(usage.total());
    if used > config.security.daily_token_budget {
        return Err(StoreError::BudgetExceeded {
            used,
            limit: config.security.daily_token_budget,
        });
    }
    Ok(response.text.unwrap_or_default())
}

/// Cài đặt compaction **một lần** cho mọi `Store`: đo ngân sách, chọn ranh giới an toàn,
/// tóm tắt phần cũ rồi xoá nó.
///
/// Best-effort: mọi lỗi ở bước tóm tắt chỉ ghi log và **giữ nguyên** lịch sử — compaction
/// không bao giờ được làm hỏng run đang chạy (agents.md mục 0.8, 8.3).
async fn compact_via(
    store: &dyn Store,
    session: SessionId,
    llm: &dyn LlmProvider,
    config: &Config,
) -> Result<(), StoreError> {
    let budget = u64::from(config.agent.context_budget_tokens);
    let trigger = budget.saturating_mul(COMPACT_TRIGGER_PERCENT) / 100;

    let stored = store.list_messages(session, None, 0).await?;
    let messages: Vec<Message> = stored.iter().map(|row| row.message.clone()).collect();
    if estimate_tokens(&messages) <= trigger {
        return Ok(());
    }

    let Some(start) = find_compaction_start(&messages, COMPACT_KEEP_RECENT) else {
        // Không có ranh giới an toàn (hiếm): giữ nguyên còn hơn làm hỏng cặp tool.
        tracing::debug!(session = %session, "compaction: không tìm được ranh giới an toàn");
        return Ok(());
    };
    if start == 0 {
        return Ok(());
    }
    debug_assert!(check_no_orphan_result(&messages, start));

    let Some(cut) = stored.get(start) else {
        return Ok(());
    };
    let previous = store.summary(session).await?;
    let summary = match summarize(store, llm, config, previous.as_deref(), &messages[..start]).await
    {
        Ok(text) => text,
        Err(error @ StoreError::BudgetExceeded { .. }) => return Err(error),
        Err(err) => {
            tracing::warn!(session = %session, error = %err, "compaction thất bại — giữ nguyên lịch sử");
            return Ok(());
        }
    };
    if summary.trim().is_empty() {
        return Ok(());
    }

    store.save_summary(session, summary.trim()).await?;
    store.delete_before(session, cut.seq).await?;
    tracing::info!(
        session = %session,
        removed = start,
        kept = messages.len().saturating_sub(start),
        "đã nén lịch sử hội thoại"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Bản SQLite + FTS5 (M5)
// ---------------------------------------------------------------------------

/// Schema v1 (agents.md mục 8.1). Idempotent để chạy lại an toàn.
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sessions (
  id INTEGER PRIMARY KEY,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  archived INTEGER NOT NULL DEFAULT 0,
  summary TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_chat ON sessions(channel, chat_id, archived);

CREATE TABLE IF NOT EXISTS messages (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL,
  role TEXT NOT NULL,
  content_json TEXT NOT NULL,
  text_for_search TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  UNIQUE(session_id, seq)
);
CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id, seq);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  text_for_search,
  content='messages',
  content_rowid='id',
  tokenize = \"unicode61 remove_diacritics 2\"
);
CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, text_for_search) VALUES (new.id, new.text_for_search);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text_for_search) VALUES ('delete', old.id, old.text_for_search);
END;
CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, text_for_search) VALUES ('delete', old.id, old.text_for_search);
  INSERT INTO messages_fts(rowid, text_for_search) VALUES (new.id, new.text_for_search);
END;

CREATE TABLE IF NOT EXISTS memories (
  id INTEGER PRIMARY KEY,
  text TEXT NOT NULL,
  tags TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
  text,
  tags,
  content='memories',
  content_rowid='id',
  tokenize = \"unicode61 remove_diacritics 2\"
);
CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
  INSERT INTO memories_fts(rowid, text, tags) VALUES (new.id, new.text, new.tags);
END;
CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, text, tags) VALUES ('delete', old.id, old.text, old.tags);
END;
CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, text, tags) VALUES ('delete', old.id, old.text, old.tags);
  INSERT INTO memories_fts(rowid, text, tags) VALUES (new.id, new.text, new.tags);
END;

CREATE TABLE IF NOT EXISTS scheduled_tasks (
  id INTEGER PRIMARY KEY,
  cron TEXT NOT NULL,
  prompt TEXT NOT NULL,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  allowed_tools TEXT NOT NULL DEFAULT '[]',
  next_run TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS web_sessions (
  token_hash BLOB PRIMARY KEY,
  user_id TEXT NOT NULL DEFAULT 'web:admin',
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  last_seen TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS usage (
  day TEXT PRIMARY KEY,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS web_sessions_expiry ON web_sessions(expires_at);
";

/// Pragma bắt buộc (agents.md mục 8.1): WAL cho đọc/ghi song song, khoá ngoại bật.
const PRAGMA_SQL: &str = "\
PRAGMA journal_mode = WAL;\
PRAGMA foreign_keys = ON;\
PRAGMA synchronous = NORMAL;";

/// Kênh trả lời của một lệnh gửi cho worker.
type Reply<T> = oneshot::Sender<Result<T, StoreError>>;

/// Lệnh gửi cho worker thread.
///
/// Mọi truy cập SQLite đi qua **một** thread sở hữu connection (agents.md mục 22.8):
/// `rusqlite` là API blocking nên không được gọi trực tiếp trong async.
enum DbCommand {
    EnsureSession {
        channel: String,
        chat_id: String,
        user_id: String,
        title: String,
        reply: Reply<SessionId>,
    },
    CreateSession {
        channel: String,
        chat_id: String,
        user_id: String,
        title: String,
        reply: Reply<SessionId>,
    },
    ArchiveSession {
        session: SessionId,
        reply: Reply<()>,
    },
    Append {
        session: SessionId,
        message: Message,
        reply: Reply<i64>,
    },
    History {
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
        reply: Reply<Vec<Message>>,
    },
    Count {
        session: SessionId,
        reply: Reply<u64>,
    },
    Clear {
        session: SessionId,
        reply: Reply<()>,
    },
    SessionInfo {
        session: SessionId,
        reply: Reply<Option<SessionInfo>>,
    },
    SaveSummary {
        session: SessionId,
        summary: String,
        reply: Reply<()>,
    },
    Summary {
        session: SessionId,
        reply: Reply<Option<String>>,
    },
    SessionSummary {
        session: SessionId,
        reply: Reply<Option<SessionSummary>>,
    },
    FindActiveSession {
        channel: String,
        chat_id: String,
        reply: Reply<Option<SessionId>>,
    },
    UpdateSession {
        session: SessionId,
        user_id: String,
        title: Option<String>,
        archived: Option<bool>,
        reply: Reply<bool>,
    },
    DeleteSession {
        session: SessionId,
        user_id: String,
        reply: Reply<bool>,
    },
    ListSessions {
        user_id: String,
        query: Option<String>,
        archived: Option<bool>,
        limit: usize,
        reply: Reply<Vec<SessionSummary>>,
    },
    MessageById {
        id: i64,
        reply: Reply<Option<MessageRecord>>,
    },
    ListMessageRecords {
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
        reply: Reply<Vec<MessageRecord>>,
    },
    ListMemories {
        query: Option<String>,
        limit: usize,
        reply: Reply<Vec<MemoryRecord>>,
    },
    DeleteMemory {
        id: u64,
        reply: Reply<bool>,
    },
    ListTasks {
        reply: Reply<Vec<ScheduledTask>>,
    },
    ListTasksForSession {
        session: SessionId,
        reply: Reply<Vec<ScheduledTask>>,
    },
    CreateTask {
        task: NewScheduledTask,
        reply: Reply<ScheduledTask>,
    },
    SetTaskEnabled {
        id: u64,
        enabled: bool,
        reply: Reply<Option<ScheduledTask>>,
    },
    DeleteTaskForSession {
        id: u64,
        session: SessionId,
        reply: Reply<bool>,
    },
    DeleteTask {
        id: u64,
        reply: Reply<bool>,
    },
    DueTasks {
        now: String,
        limit: usize,
        reply: Reply<Vec<ScheduledTask>>,
    },
    ClaimTask {
        id: u64,
        expected_next_run: String,
        next_run: String,
        last_run_at: String,
        status: String,
        reply: Reply<Option<ScheduledTask>>,
    },
    SetTaskStatus {
        id: u64,
        status: String,
        reply: Reply<bool>,
    },
    AddUsage {
        day: String,
        usage: Usage,
        reply: Reply<()>,
    },
    Usage {
        day: String,
        reply: Reply<Usage>,
    },
    CreateWebSession {
        token_hash: Vec<u8>,
        user_id: String,
        created_at: String,
        expires_at: String,
        reply: Reply<()>,
    },
    GetWebSession {
        token_hash: Vec<u8>,
        now: String,
        reply: Reply<Option<WebSessionInfo>>,
    },
    TouchWebSession {
        token_hash: Vec<u8>,
        now: String,
        reply: Reply<bool>,
    },
    DeleteWebSession {
        token_hash: Vec<u8>,
        reply: Reply<bool>,
    },
    DeleteAllWebSessions {
        reply: Reply<()>,
    },
    MemorySave {
        text: String,
        tags: String,
        reply: Reply<u64>,
    },
    MemorySearch {
        query: String,
        reply: Reply<Vec<MemorySearchHit>>,
    },
    Outbound {
        channel: String,
        chat_id: String,
        payload: Outbound,
        next_attempt_at: String,
        reply: Reply<u64>,
    },
    DueOutbox {
        now: String,
        limit: usize,
        reply: Reply<Vec<OutboxEntry>>,
    },
    CompleteOutbox {
        id: u64,
        reply: Reply<()>,
    },
    RetryOutbox {
        id: u64,
        next_attempt_at: String,
        last_error: String,
        reply: Reply<()>,
    },
    ListMessages {
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
        reply: Reply<Vec<StoredMessage>>,
    },
    DeleteBefore {
        session: SessionId,
        before_seq: u64,
        reply: Reply<()>,
    },
}

/// Trạng thái dùng chung của [`SqliteStore`] (chia sẻ qua `Arc` để `SqliteStore` clone được).
struct SqliteInner {
    tx: Mutex<Option<std::sync::mpsc::Sender<DbCommand>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for SqliteInner {
    fn drop(&mut self) {
        // Đóng kênh để `worker_loop` thoát khỏi `recv()`, rồi join để connection được
        // đóng (flush WAL) trước khi tiến trình dừng.
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
        if let Ok(mut worker) = self.worker.lock()
            && let Some(handle) = worker.take()
            && handle.join().is_err()
        {
            tracing::warn!("worker bộ nhớ kết thúc bất thường");
        }
    }
}

/// Store bền vững trên SQLite + FTS5 (agents.md mục 8).
///
/// `SqliteStore` chỉ giữ kênh gửi lệnh; connection thật nằm ở thread worker riêng nên
/// mọi hàm `async` ở đây đều **không** block runtime.
#[derive(Clone)]
pub struct SqliteStore {
    inner: Arc<SqliteInner>,
}

impl SqliteStore {
    /// Mở (hoặc tạo) database ở `path`, chạy migration rồi khởi động worker thread.
    ///
    /// # Errors
    /// [`StoreError::Internal`] khi không tạo được thư mục/mở file/migration lỗi hoặc
    /// không tạo được worker.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                StoreError::Internal(format!(
                    "tạo thư mục dữ liệu {} thất bại: {err}",
                    parent.display()
                ))
            })?;
        }
        let conn = Connection::open(path).map_err(|err| {
            StoreError::Internal(format!("mở SQLite {} thất bại: {err}", path.display()))
        })?;
        configure(&conn)?;
        run_migration(&conn)?;

        let (tx, rx) = std::sync::mpsc::channel::<DbCommand>();
        let worker = std::thread::Builder::new()
            .name("beanagent-memory-worker".to_string())
            .spawn(move || worker_loop(conn, rx))
            .map_err(|err| StoreError::Internal(format!("không tạo được worker bộ nhớ: {err}")))?;

        Ok(Self {
            inner: Arc::new(SqliteInner {
                tx: Mutex::new(Some(tx)),
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    /// Gửi lệnh cho worker và chờ kết quả.
    async fn request<T>(&self, build: impl FnOnce(Reply<T>) -> DbCommand) -> Result<T, StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            // Khoá **không** được giữ qua `.await` (agents.md mục 22.7).
            let guard = self
                .inner
                .tx
                .lock()
                .map_err(|_| StoreError::Internal("khoá kênh worker bị hỏng".into()))?;
            let Some(sender) = guard.as_ref() else {
                return Err(StoreError::Internal("store đã đóng".into()));
            };
            sender
                .send(build(reply_tx))
                .map_err(|_| StoreError::Internal("worker bộ nhớ đã dừng".into()))?;
        }
        reply_rx
            .await
            .map_err(|_| StoreError::Internal("worker bộ nhớ không phản hồi".into()))?
    }
}

#[async_trait::async_trait]
impl Store for SqliteStore {
    async fn append(&self, session: SessionId, msg: Message) -> Result<i64, StoreError> {
        self.request(move |reply| DbCommand::Append {
            session,
            message: msg,
            reply,
        })
        .await
    }

    async fn history(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Message>, StoreError> {
        self.request(move |reply| DbCommand::History {
            session,
            before_seq,
            limit,
            reply,
        })
        .await
    }

    async fn count(&self, session: SessionId) -> Result<u64, StoreError> {
        self.request(move |reply| DbCommand::Count { session, reply })
            .await
    }

    async fn clear(&self, session: SessionId) -> Result<(), StoreError> {
        self.request(move |reply| DbCommand::Clear { session, reply })
            .await
    }

    async fn session_info(&self, session: SessionId) -> Result<Option<SessionInfo>, StoreError> {
        self.request(move |reply| DbCommand::SessionInfo { session, reply })
            .await
    }

    async fn ensure_session(
        &self,
        channel: &str,
        chat_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError> {
        let channel = channel.to_string();
        let chat_id = chat_id.to_string();
        let title = title.to_string();
        self.request(move |reply| DbCommand::EnsureSession {
            channel,
            chat_id,
            user_id: String::new(),
            title,
            reply,
        })
        .await
    }

    async fn ensure_session_for_user(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError> {
        let channel = channel.to_string();
        let chat_id = chat_id.to_string();
        let user_id = user_id.to_string();
        let title = title.to_string();
        self.request(move |reply| DbCommand::EnsureSession {
            channel,
            chat_id,
            user_id,
            title,
            reply,
        })
        .await
    }

    async fn archive_session(&self, session: SessionId) -> Result<(), StoreError> {
        self.request(move |reply| DbCommand::ArchiveSession { session, reply })
            .await
    }

    async fn save_summary(&self, session: SessionId, summary: &str) -> Result<(), StoreError> {
        let summary = summary.to_string();
        self.request(move |reply| DbCommand::SaveSummary {
            session,
            summary,
            reply,
        })
        .await
    }

    async fn summary(&self, session: SessionId) -> Result<Option<String>, StoreError> {
        self.request(move |reply| DbCommand::Summary { session, reply })
            .await
    }

    async fn create_session(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        title: &str,
    ) -> Result<SessionId, StoreError> {
        let channel = channel.to_string();
        let chat_id = chat_id.to_string();
        let user_id = user_id.to_string();
        let title = title.to_string();
        self.request(move |reply| DbCommand::CreateSession {
            channel,
            chat_id,
            user_id,
            title,
            reply,
        })
        .await
    }

    async fn session_summary(
        &self,
        session: SessionId,
    ) -> Result<Option<SessionSummary>, StoreError> {
        self.request(move |reply| DbCommand::SessionSummary { session, reply })
            .await
    }

    async fn update_session(
        &self,
        session: SessionId,
        user_id: &str,
        title: Option<&str>,
        archived: Option<bool>,
    ) -> Result<bool, StoreError> {
        let user_id = user_id.to_string();
        let title = title.map(str::to_string);
        self.request(move |reply| DbCommand::UpdateSession {
            session,
            user_id,
            title,
            archived,
            reply,
        })
        .await
    }

    async fn delete_session(&self, session: SessionId, user_id: &str) -> Result<bool, StoreError> {
        let user_id = user_id.to_string();
        self.request(move |reply| DbCommand::DeleteSession {
            session,
            user_id,
            reply,
        })
        .await
    }

    async fn list_sessions(
        &self,
        user_id: &str,
        query: Option<&str>,
        archived: Option<bool>,
        limit: usize,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let user_id = user_id.to_string();
        let query = query.map(str::to_string);
        self.request(move |reply| DbCommand::ListSessions {
            user_id,
            query,
            archived,
            limit,
            reply,
        })
        .await
    }

    async fn message_by_id(&self, id: i64) -> Result<Option<MessageRecord>, StoreError> {
        self.request(move |reply| DbCommand::MessageById { id, reply })
            .await
    }

    async fn list_message_records(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<MessageRecord>, StoreError> {
        self.request(move |reply| DbCommand::ListMessageRecords {
            session,
            before_seq,
            limit,
            reply,
        })
        .await
    }

    async fn list_memories(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>, StoreError> {
        let query = query.map(str::to_string);
        self.request(move |reply| DbCommand::ListMemories {
            query,
            limit,
            reply,
        })
        .await
    }

    async fn delete_memory(&self, id: u64) -> Result<bool, StoreError> {
        self.request(move |reply| DbCommand::DeleteMemory { id, reply })
            .await
    }

    async fn find_active_session(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Option<SessionId>, StoreError> {
        let channel = channel.to_string();
        let chat_id = chat_id.to_string();
        self.request(move |reply| DbCommand::FindActiveSession {
            channel,
            chat_id,
            reply,
        })
        .await
    }

    async fn list_tasks(&self) -> Result<Vec<ScheduledTask>, StoreError> {
        self.request(|reply| DbCommand::ListTasks { reply }).await
    }

    async fn list_tasks_for_session(
        &self,
        session: SessionId,
    ) -> Result<Vec<ScheduledTask>, StoreError> {
        self.request(move |reply| DbCommand::ListTasksForSession { session, reply })
            .await
    }

    async fn create_task(&self, task: NewScheduledTask) -> Result<ScheduledTask, StoreError> {
        self.request(move |reply| DbCommand::CreateTask { task, reply })
            .await
    }

    async fn set_task_enabled(
        &self,
        id: u64,
        enabled: bool,
    ) -> Result<Option<ScheduledTask>, StoreError> {
        self.request(move |reply| DbCommand::SetTaskEnabled { id, enabled, reply })
            .await
    }

    async fn delete_task_for_session(
        &self,
        id: u64,
        session: SessionId,
    ) -> Result<bool, StoreError> {
        self.request(move |reply| DbCommand::DeleteTaskForSession { id, session, reply })
            .await
    }

    async fn delete_task(&self, id: u64) -> Result<bool, StoreError> {
        self.request(move |reply| DbCommand::DeleteTask { id, reply })
            .await
    }

    async fn due_tasks(&self, now: &str, limit: usize) -> Result<Vec<ScheduledTask>, StoreError> {
        let now = now.to_string();
        self.request(move |reply| DbCommand::DueTasks { now, limit, reply })
            .await
    }

    async fn claim_task(
        &self,
        id: u64,
        expected_next_run: &str,
        next_run: &str,
        last_run_at: &str,
        status: &str,
    ) -> Result<Option<ScheduledTask>, StoreError> {
        let expected_next_run = expected_next_run.to_string();
        let next_run = next_run.to_string();
        let last_run_at = last_run_at.to_string();
        let status = status.to_string();
        self.request(move |reply| DbCommand::ClaimTask {
            id,
            expected_next_run,
            next_run,
            last_run_at,
            status,
            reply,
        })
        .await
    }

    async fn set_task_status(&self, id: u64, status: &str) -> Result<bool, StoreError> {
        let status = status.to_string();
        self.request(move |reply| DbCommand::SetTaskStatus { id, status, reply })
            .await
    }

    async fn add_usage(&self, day: &str, usage: Usage) -> Result<(), StoreError> {
        let day = day.to_string();
        self.request(move |reply| DbCommand::AddUsage { day, usage, reply })
            .await
    }

    async fn usage(&self, day: &str) -> Result<Usage, StoreError> {
        let day = day.to_string();
        self.request(move |reply| DbCommand::Usage { day, reply })
            .await
    }

    async fn create_web_session(
        &self,
        token_hash: Vec<u8>,
        user_id: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), StoreError> {
        let user_id = user_id.to_string();
        let created_at = created_at.to_string();
        let expires_at = expires_at.to_string();
        self.request(move |reply| DbCommand::CreateWebSession {
            token_hash,
            user_id,
            created_at,
            expires_at,
            reply,
        })
        .await
    }

    async fn get_web_session(
        &self,
        token_hash: &[u8],
        now: &str,
    ) -> Result<Option<WebSessionInfo>, StoreError> {
        let token_hash = token_hash.to_vec();
        let now = now.to_string();
        self.request(move |reply| DbCommand::GetWebSession {
            token_hash,
            now,
            reply,
        })
        .await
    }

    async fn touch_web_session(&self, token_hash: &[u8], now: &str) -> Result<bool, StoreError> {
        let token_hash = token_hash.to_vec();
        let now = now.to_string();
        self.request(move |reply| DbCommand::TouchWebSession {
            token_hash,
            now,
            reply,
        })
        .await
    }

    async fn delete_web_session(&self, token_hash: &[u8]) -> Result<bool, StoreError> {
        let token_hash = token_hash.to_vec();
        self.request(move |reply| DbCommand::DeleteWebSession { token_hash, reply })
            .await
    }

    async fn delete_all_web_sessions(&self) -> Result<(), StoreError> {
        self.request(|reply| DbCommand::DeleteAllWebSessions { reply })
            .await
    }

    async fn memory_save(&self, text: &str, tags: &str) -> Result<u64, StoreError> {
        let text = text.to_string();
        let tags = tags.to_string();
        self.request(move |reply| DbCommand::MemorySave { text, tags, reply })
            .await
    }

    async fn memory_search(&self, query: &str) -> Result<Vec<MemorySearchHit>, StoreError> {
        let query = query.to_string();
        self.request(move |reply| DbCommand::MemorySearch { query, reply })
            .await
    }

    async fn enqueue_outbound(
        &self,
        channel: &str,
        chat_id: &str,
        payload: &Outbound,
        next_attempt_at: &str,
    ) -> Result<u64, StoreError> {
        let channel = channel.to_string();
        let chat_id = chat_id.to_string();
        let payload = payload.clone();
        let next_attempt_at = next_attempt_at.to_string();
        self.request(move |reply| DbCommand::Outbound {
            channel,
            chat_id,
            payload,
            next_attempt_at,
            reply,
        })
        .await
    }

    async fn due_outbox(&self, now: &str, limit: usize) -> Result<Vec<OutboxEntry>, StoreError> {
        let now = now.to_string();
        self.request(move |reply| DbCommand::DueOutbox { now, limit, reply })
            .await
    }

    async fn complete_outbox(&self, id: u64) -> Result<(), StoreError> {
        self.request(move |reply| DbCommand::CompleteOutbox { id, reply })
            .await
    }

    async fn retry_outbox(
        &self,
        id: u64,
        next_attempt_at: &str,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let next_attempt_at = next_attempt_at.to_string();
        let last_error = last_error.to_string();
        self.request(move |reply| DbCommand::RetryOutbox {
            id,
            next_attempt_at,
            last_error,
            reply,
        })
        .await
    }

    async fn list_messages(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, StoreError> {
        self.request(move |reply| DbCommand::ListMessages {
            session,
            before_seq,
            limit,
            reply,
        })
        .await
    }

    async fn delete_before(&self, session: SessionId, before_seq: u64) -> Result<(), StoreError> {
        self.request(move |reply| DbCommand::DeleteBefore {
            session,
            before_seq,
            reply,
        })
        .await
    }

    async fn compact(
        &self,
        session: SessionId,
        llm: &dyn LlmProvider,
        config: &Config,
    ) -> Result<(), StoreError> {
        let store: &dyn Store = self;
        compact_via(store, session, llm, config).await
    }
}

// ---------------------------------------------------------------------------
// Worker thread — nơi duy nhất chạm vào SQLite
// ---------------------------------------------------------------------------

fn worker_loop(mut conn: Connection, rx: std::sync::mpsc::Receiver<DbCommand>) {
    while let Ok(cmd) = rx.recv() {
        dispatch(&mut conn, cmd);
    }
    tracing::debug!("worker bộ nhớ đã dừng");
}

/// Thực thi một lệnh và gửi kết quả về; lỗi SQLite **không** làm chết worker.
fn dispatch(conn: &mut Connection, cmd: DbCommand) {
    match cmd {
        DbCommand::EnsureSession {
            channel,
            chat_id,
            user_id,
            title,
            reply,
        } => {
            let _ = reply.send(ensure_session(conn, &channel, &chat_id, &user_id, &title));
        }
        DbCommand::CreateSession {
            channel,
            chat_id,
            user_id,
            title,
            reply,
        } => {
            let _ = reply.send(create_session_row(
                conn, &channel, &chat_id, &user_id, &title,
            ));
        }
        DbCommand::ArchiveSession { session, reply } => {
            let _ = reply.send(archive_session(conn, session));
        }
        DbCommand::Append {
            session,
            message,
            reply,
        } => {
            let _ = reply.send(append_message(conn, session, &message));
        }
        DbCommand::History {
            session,
            before_seq,
            limit,
            reply,
        } => {
            let _ = reply.send(load_history(conn, session, before_seq, limit));
        }
        DbCommand::Count { session, reply } => {
            let _ = reply.send(count_messages(conn, session));
        }
        DbCommand::Clear { session, reply } => {
            let _ = reply.send(clear_messages(conn, session));
        }
        DbCommand::SessionInfo { session, reply } => {
            let _ = reply.send(load_session_info(conn, session));
        }
        DbCommand::SaveSummary {
            session,
            summary,
            reply,
        } => {
            let _ = reply.send(save_summary(conn, session, &summary));
        }
        DbCommand::Summary { session, reply } => {
            let _ = reply.send(load_summary(conn, session));
        }
        DbCommand::SessionSummary { session, reply } => {
            let _ = reply.send(load_session_summary(conn, session));
        }
        DbCommand::FindActiveSession {
            channel,
            chat_id,
            reply,
        } => {
            let _ = reply.send(find_active_session(conn, &channel, &chat_id));
        }
        DbCommand::UpdateSession {
            session,
            user_id,
            title,
            archived,
            reply,
        } => {
            let _ = reply.send(update_session(
                conn,
                session,
                &user_id,
                title.as_deref(),
                archived,
            ));
        }
        DbCommand::DeleteSession {
            session,
            user_id,
            reply,
        } => {
            let _ = reply.send(delete_session(conn, session, &user_id));
        }
        DbCommand::ListSessions {
            user_id,
            query,
            archived,
            limit,
            reply,
        } => {
            let _ = reply.send(list_sessions(
                conn,
                &user_id,
                query.as_deref(),
                archived,
                limit,
            ));
        }
        DbCommand::MessageById { id, reply } => {
            let _ = reply.send(message_by_id(conn, id));
        }
        DbCommand::ListMessageRecords {
            session,
            before_seq,
            limit,
            reply,
        } => {
            let _ = reply.send(list_message_records(conn, session, before_seq, limit));
        }
        DbCommand::ListMemories {
            query,
            limit,
            reply,
        } => {
            let _ = reply.send(list_memories(conn, query.as_deref(), limit));
        }
        DbCommand::DeleteMemory { id, reply } => {
            let _ = reply.send(delete_memory(conn, id));
        }
        DbCommand::ListTasks { reply } => {
            let _ = reply.send(list_tasks(conn));
        }
        DbCommand::ListTasksForSession { session, reply } => {
            let _ = reply.send(list_tasks_for_session(conn, session));
        }
        DbCommand::CreateTask { task, reply } => {
            let _ = reply.send(create_task(conn, task));
        }
        DbCommand::SetTaskEnabled { id, enabled, reply } => {
            let _ = reply.send(set_task_enabled(conn, id, enabled));
        }
        DbCommand::DeleteTaskForSession { id, session, reply } => {
            let _ = reply.send(delete_task_for_session(conn, id, session));
        }
        DbCommand::DeleteTask { id, reply } => {
            let _ = reply.send(delete_task(conn, id));
        }
        DbCommand::DueTasks { now, limit, reply } => {
            let _ = reply.send(due_tasks(conn, &now, limit));
        }
        DbCommand::ClaimTask {
            id,
            expected_next_run,
            next_run,
            last_run_at,
            status,
            reply,
        } => {
            let _ = reply.send(claim_task(
                conn,
                id,
                &expected_next_run,
                &next_run,
                &last_run_at,
                &status,
            ));
        }
        DbCommand::SetTaskStatus { id, status, reply } => {
            let _ = reply.send(set_task_status(conn, id, &status));
        }
        DbCommand::AddUsage { day, usage, reply } => {
            let _ = reply.send(add_usage(conn, &day, usage));
        }
        DbCommand::Usage { day, reply } => {
            let _ = reply.send(read_usage(conn, &day));
        }
        DbCommand::CreateWebSession {
            token_hash,
            user_id,
            created_at,
            expires_at,
            reply,
        } => {
            let _ = reply.send(create_web_session(
                conn,
                &token_hash,
                &user_id,
                &created_at,
                &expires_at,
            ));
        }
        DbCommand::GetWebSession {
            token_hash,
            now,
            reply,
        } => {
            let _ = reply.send(get_web_session(conn, &token_hash, &now));
        }
        DbCommand::TouchWebSession {
            token_hash,
            now,
            reply,
        } => {
            let _ = reply.send(touch_web_session(conn, &token_hash, &now));
        }
        DbCommand::DeleteWebSession { token_hash, reply } => {
            let _ = reply.send(delete_web_session(conn, &token_hash));
        }
        DbCommand::DeleteAllWebSessions { reply } => {
            let _ = reply.send(delete_all_web_sessions(conn));
        }
        DbCommand::MemorySave { text, tags, reply } => {
            let _ = reply.send(memory_save_row(conn, &text, &tags));
        }
        DbCommand::MemorySearch { query, reply } => {
            let _ = reply.send(memory_search(conn, &query));
        }
        DbCommand::Outbound {
            channel,
            chat_id,
            payload,
            next_attempt_at,
            reply,
        } => {
            let _ = reply.send(insert_outbound(
                conn,
                &channel,
                &chat_id,
                &payload,
                &next_attempt_at,
            ));
        }
        DbCommand::DueOutbox { now, limit, reply } => {
            let _ = reply.send(load_due_outbox(conn, &now, limit));
        }
        DbCommand::CompleteOutbox { id, reply } => {
            let _ = reply.send(delete_outbox(conn, id));
        }
        DbCommand::RetryOutbox {
            id,
            next_attempt_at,
            last_error,
            reply,
        } => {
            let _ = reply.send(update_outbox_retry(conn, id, &next_attempt_at, &last_error));
        }
        DbCommand::ListMessages {
            session,
            before_seq,
            limit,
            reply,
        } => {
            let _ = reply.send(list_messages(conn, session, before_seq, limit));
        }
        DbCommand::DeleteBefore {
            session,
            before_seq,
            reply,
        } => {
            let _ = reply.send(delete_before(conn, session, before_seq));
        }
    }
}

// ---------------------------------------------------------------------------
// Pragma, migration, tiện ích
// ---------------------------------------------------------------------------

fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(PRAGMA_SQL).map_err(internal)?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(internal)?;
    Ok(())
}

/// Migration tuần tự theo `user_version` (agents.md mục 8.1).
fn run_migration(conn: &Connection) -> Result<(), StoreError> {
    let mut version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(internal)?;
    if version < 1 {
        conn.execute_batch(SCHEMA_SQL).map_err(|err| {
            StoreError::Internal(format!(
                "migration v1 thất bại (thiếu FTS5 trong SQLite?): {err}"
            ))
        })?;
        conn.pragma_update(None, "user_version", 1)
            .map_err(internal)?;
        version = 1;
    }
    if version < 2 {
        add_column_if_missing(conn, "sessions", "user_id", "TEXT NOT NULL DEFAULT ''")?;
        add_column_if_missing(
            conn,
            "outbox",
            "next_attempt_at",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(conn, "outbox", "last_error", "TEXT NOT NULL DEFAULT ''")?;
        conn.execute_batch(
            "UPDATE outbox SET next_attempt_at = created_at WHERE next_attempt_at = '';
             CREATE INDEX IF NOT EXISTS outbox_due ON outbox(next_attempt_at, id);",
        )
        .map_err(|err| StoreError::Internal(format!("migration v2 outbox thất bại: {err}")))?;
        conn.pragma_update(None, "user_version", 2)
            .map_err(internal)?;
        version = 2;
    }
    if version < 3 {
        add_column_if_missing(
            conn,
            "web_sessions",
            "user_id",
            "TEXT NOT NULL DEFAULT 'web:admin'",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage (
               day TEXT PRIMARY KEY,
               input_tokens INTEGER NOT NULL DEFAULT 0,
               output_tokens INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS web_sessions_expiry ON web_sessions(expires_at);",
        )
        .map_err(|err| StoreError::Internal(format!("migration v3 web/usage thất bại: {err}")))?;
        conn.pragma_update(None, "user_version", 3)
            .map_err(internal)?;
        version = 3;
    }
    if version < 4 {
        add_column_if_missing(
            conn,
            "scheduled_tasks",
            "session_id",
            "INTEGER REFERENCES sessions(id)",
        )?;
        add_column_if_missing(
            conn,
            "scheduled_tasks",
            "created_at",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(conn, "scheduled_tasks", "last_run_at", "TEXT")?;
        add_column_if_missing(
            conn,
            "scheduled_tasks",
            "last_status",
            "TEXT NOT NULL DEFAULT 'pending'",
        )?;
        conn.execute_batch(
            "UPDATE scheduled_tasks
                SET created_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
              WHERE created_at = '';
             CREATE INDEX IF NOT EXISTS scheduled_tasks_due
               ON scheduled_tasks(enabled, next_run, id);",
        )
        .map_err(|err| StoreError::Internal(format!("migration v4 scheduler thất bại: {err}")))?;
        conn.pragma_update(None, "user_version", 4)
            .map_err(internal)?;
    }
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), StoreError> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(internal)?;
    let mut rows = statement.query([]).map_err(internal)?;
    while let Some(row) = rows.next().map_err(internal)? {
        let name: String = row.get(1).map_err(internal)?;
        if name == column {
            return Ok(());
        }
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
    .map_err(|err| {
        StoreError::Internal(format!(
            "migration thêm cột {table}.{column} thất bại: {err}"
        ))
    })
}

/// Bọc lỗi SQLite; nội dung lỗi chỉ chứa mã/khai báo SQL, **không** chứa dữ liệu hội thoại.
fn internal(err: rusqlite::Error) -> StoreError {
    StoreError::Internal(format!("SQLite: {err}"))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn decode_message(json: &str) -> Result<Message, StoreError> {
    serde_json::from_str(json)
        .map_err(|err| StoreError::Internal(format!("message trong DB không đọc được: {err}")))
}

fn load_session_info(
    conn: &Connection,
    session: SessionId,
) -> Result<Option<SessionInfo>, StoreError> {
    conn.query_row(
        "SELECT channel, chat_id, user_id, archived FROM sessions WHERE id = ?1",
        params![session.get()],
        |row| {
            Ok(SessionInfo {
                id: session,
                channel: row.get(0)?,
                chat_id: row.get(1)?,
                user_id: row.get(2)?,
                archived: row.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()
    .map_err(internal)
}

fn load_session_summary(
    conn: &Connection,
    session: SessionId,
) -> Result<Option<SessionSummary>, StoreError> {
    conn.query_row(
        "SELECT id, channel, chat_id, user_id, title, archived, created_at, updated_at \
         FROM sessions WHERE id = ?1",
        params![session.get()],
        |row| {
            Ok(SessionSummary {
                id: SessionId::new(row.get(0)?),
                channel: row.get(1)?,
                chat_id: row.get(2)?,
                user_id: row.get(3)?,
                title: row.get(4)?,
                archived: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        },
    )
    .optional()
    .map_err(internal)
}

fn update_session(
    conn: &Connection,
    session: SessionId,
    user_id: &str,
    title: Option<&str>,
    archived: Option<bool>,
) -> Result<bool, StoreError> {
    let now = now_rfc3339();
    let changed = conn.execute(
        "UPDATE sessions SET title = COALESCE(?2, title), archived = COALESCE(?3, archived), updated_at = ?4 \
         WHERE id = ?1 AND user_id = ?5",
        params![session.get(), title, archived.map(i64::from), now, user_id],
    ).map_err(internal)?;
    Ok(changed > 0)
}

fn delete_session(
    conn: &Connection,
    session: SessionId,
    user_id: &str,
) -> Result<bool, StoreError> {
    let changed = conn
        .execute(
            "DELETE FROM sessions WHERE id = ?1 AND user_id = ?2",
            params![session.get(), user_id],
        )
        .map_err(internal)?;
    Ok(changed > 0)
}

fn list_sessions(
    conn: &Connection,
    user_id: &str,
    query: Option<&str>,
    archived: Option<bool>,
    limit: usize,
) -> Result<Vec<SessionSummary>, StoreError> {
    let query = query.unwrap_or_default();
    let pattern = format!("%{query}%");
    let mut stmt = conn
        .prepare(
            "SELECT id, channel, chat_id, user_id, title, archived, created_at, updated_at \
         FROM sessions WHERE user_id = ?1 AND (?2 IS NULL OR archived = ?2) \
         AND (?3 = '' OR title LIKE ?4) ORDER BY updated_at DESC LIMIT ?5",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(
            params![
                user_id,
                archived.map(i64::from),
                query,
                pattern,
                sql_limit(limit)
            ],
            |row| {
                Ok(SessionSummary {
                    id: SessionId::new(row.get(0)?),
                    channel: row.get(1)?,
                    chat_id: row.get(2)?,
                    user_id: row.get(3)?,
                    title: row.get(4)?,
                    archived: row.get::<_, i64>(5)? != 0,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

fn message_by_id(conn: &Connection, id: i64) -> Result<Option<MessageRecord>, StoreError> {
    conn.query_row(
        "SELECT session_id, seq, content_json, created_at FROM messages WHERE id = ?1",
        params![id],
        |row| {
            let session_id: i64 = row.get(0)?;
            let seq: i64 = row.get(1)?;
            let content: String = row.get(2)?;
            Ok((session_id, seq, content, row.get::<_, String>(3)?))
        },
    )
    .optional()
    .map_err(internal)?
    .map(|(session_id, seq, content, created_at)| {
        Ok(MessageRecord {
            id,
            session_id: SessionId::new(session_id),
            seq: u64::try_from(seq).map_err(|_| StoreError::Internal("seq âm".into()))?,
            message: decode_message(&content)?,
            created_at,
        })
    })
    .transpose()
}

fn list_message_records(
    conn: &Connection,
    session: SessionId,
    before_seq: Option<u64>,
    limit: usize,
) -> Result<Vec<MessageRecord>, StoreError> {
    let before = before_seq.map(|seq| seq as i64);
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, seq, content_json, created_at FROM (\
           SELECT id, session_id, seq, content_json, created_at FROM messages \
           WHERE session_id = ?1 AND (?2 IS NULL OR seq < ?2) \
           ORDER BY seq DESC LIMIT ?3\
         ) ORDER BY seq ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session.get(), before, sql_limit(limit)], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, session_id, seq, content, created_at) = row.map_err(internal)?;
        out.push(MessageRecord {
            id,
            session_id: SessionId::new(session_id),
            seq: u64::try_from(seq).map_err(|_| StoreError::Internal("seq âm".into()))?,
            message: decode_message(&content)?,
            created_at,
        });
    }
    Ok(out)
}

fn list_memories(
    conn: &Connection,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<MemoryRecord>, StoreError> {
    let query = query.unwrap_or_default();
    let pattern = format!("%{query}%");
    let mut stmt = conn
        .prepare(
            "SELECT id, text, tags, created_at FROM memories \
         WHERE ?1 = '' OR text LIKE ?2 OR tags LIKE ?2 ORDER BY id DESC LIMIT ?3",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![query, pattern, sql_limit(limit)], |row| {
            let id: i64 = row.get(0)?;
            let id = u64::try_from(id).map_err(|_| {
                rusqlite::Error::InvalidColumnType(0, "id".into(), rusqlite::types::Type::Integer)
            })?;
            Ok(MemoryRecord {
                id,
                text: row.get(1)?,
                tags: row.get(2)?,
                created_at: row.get(3)?,
            })
        })
        .map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

fn delete_memory(conn: &Connection, id: u64) -> Result<bool, StoreError> {
    let id =
        i64::try_from(id).map_err(|_| StoreError::Internal("memory id vượt giới hạn".into()))?;
    Ok(conn
        .execute("DELETE FROM memories WHERE id = ?1", params![id])
        .map_err(internal)?
        > 0)
}

fn find_active_session(
    conn: &Connection,
    channel: &str,
    chat_id: &str,
) -> Result<Option<SessionId>, StoreError> {
    conn.query_row(
        "SELECT id FROM sessions
          WHERE channel = ?1 AND chat_id = ?2 AND archived = 0
          ORDER BY id DESC LIMIT 1",
        params![channel, chat_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|id| id.map(SessionId::new))
    .map_err(internal)
}

fn task_select(where_clause: &str) -> String {
    format!(
        "SELECT id, cron, prompt, session_id, channel, chat_id, allowed_tools, next_run, \
                enabled, created_at, last_run_at, last_status
         FROM scheduled_tasks {where_clause}"
    )
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTask> {
    let id = row.get::<_, i64>(0)?;
    let allowed = row.get::<_, String>(6)?;
    let allowed_tools = serde_json::from_str::<Vec<String>>(&allowed).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let id = u64::try_from(id).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            "scheduled task id âm".into(),
        )
    })?;
    Ok(ScheduledTask {
        id,
        cron: row.get(1)?,
        prompt: row.get(2)?,
        session_id: row.get::<_, Option<i64>>(3)?.map(SessionId::new),
        channel: row.get(4)?,
        chat_id: row.get(5)?,
        allowed_tools,
        next_run: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        last_run_at: row.get(10)?,
        last_status: row.get(11)?,
    })
}

fn list_tasks(conn: &Connection) -> Result<Vec<ScheduledTask>, StoreError> {
    let mut stmt = conn
        .prepare(&task_select("ORDER BY id DESC"))
        .map_err(internal)?;
    let rows = stmt.query_map([], task_from_row).map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

fn list_tasks_for_session(
    conn: &Connection,
    session: SessionId,
) -> Result<Vec<ScheduledTask>, StoreError> {
    let mut stmt = conn
        .prepare(&task_select("WHERE session_id = ?1 ORDER BY id DESC"))
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session.get()], task_from_row)
        .map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

fn create_task(conn: &Connection, task: NewScheduledTask) -> Result<ScheduledTask, StoreError> {
    let allowed_tools = serde_json::to_string(&task.allowed_tools).map_err(|error| {
        StoreError::Internal(format!("serialize allowed_tools thất bại: {error}"))
    })?;
    let created_at = now_rfc3339();
    let last_status = if task.enabled { "pending" } else { "disabled" };
    conn.execute(
        "INSERT INTO scheduled_tasks
           (cron, prompt, session_id, channel, chat_id, allowed_tools, next_run, enabled,
            created_at, last_run_at, last_status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)",
        params![
            task.cron,
            task.prompt,
            task.session_id.map(|session| session.get()),
            task.channel,
            task.chat_id,
            allowed_tools,
            task.next_run,
            i64::from(task.enabled),
            created_at,
            last_status,
        ],
    )
    .map_err(internal)?;
    let id = conn.last_insert_rowid();
    let id = u64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id không hợp lệ".into()))?;
    Ok(ScheduledTask {
        id,
        cron: task.cron,
        prompt: task.prompt,
        session_id: task.session_id,
        channel: task.channel,
        chat_id: task.chat_id,
        allowed_tools: task.allowed_tools,
        next_run: task.next_run,
        enabled: task.enabled,
        created_at,
        last_run_at: None,
        last_status: last_status.into(),
    })
}

fn load_task(conn: &Connection, id: u64) -> Result<Option<ScheduledTask>, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    conn.query_row(
        &task_select("WHERE id = ?1"),
        params![id_i64],
        task_from_row,
    )
    .optional()
    .map_err(internal)
}

fn set_task_enabled(
    conn: &Connection,
    id: u64,
    enabled: bool,
) -> Result<Option<ScheduledTask>, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    let changed = conn
        .execute(
            "UPDATE scheduled_tasks
                SET enabled = ?2,
                    last_status = CASE
                        WHEN ?2 = 0 THEN 'disabled'
                        WHEN last_status = 'disabled' THEN 'pending'
                        ELSE last_status
                    END
              WHERE id = ?1",
            params![id_i64, i64::from(enabled)],
        )
        .map_err(internal)?;
    if changed == 0 {
        return Ok(None);
    }
    load_task(conn, id)
}

fn delete_task_for_session(
    conn: &Connection,
    id: u64,
    session: SessionId,
) -> Result<bool, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    Ok(conn
        .execute(
            "DELETE FROM scheduled_tasks WHERE id = ?1 AND session_id = ?2",
            params![id_i64, session.get()],
        )
        .map_err(internal)?
        > 0)
}

fn delete_task(conn: &Connection, id: u64) -> Result<bool, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    Ok(conn
        .execute("DELETE FROM scheduled_tasks WHERE id = ?1", params![id_i64])
        .map_err(internal)?
        > 0)
}

fn due_tasks(conn: &Connection, now: &str, limit: usize) -> Result<Vec<ScheduledTask>, StoreError> {
    let mut stmt = conn
        .prepare(&task_select(
            "WHERE enabled = 1 AND last_status != 'running' AND next_run <= ?1
             ORDER BY next_run, id LIMIT ?2",
        ))
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![now, sql_limit(limit)], task_from_row)
        .map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

fn claim_task(
    conn: &Connection,
    id: u64,
    expected_next_run: &str,
    next_run: &str,
    last_run_at: &str,
    status: &str,
) -> Result<Option<ScheduledTask>, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    let changed = conn
        .execute(
            "UPDATE scheduled_tasks
                SET next_run = ?2, last_run_at = ?3, last_status = ?4
              WHERE id = ?1 AND enabled = 1 AND next_run = ?5",
            params![id_i64, next_run, last_run_at, status, expected_next_run],
        )
        .map_err(internal)?;
    if changed == 0 {
        return Ok(None);
    }
    load_task(conn, id)
}

fn set_task_status(conn: &Connection, id: u64, status: &str) -> Result<bool, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    Ok(conn
        .execute(
            "UPDATE scheduled_tasks SET last_status = ?2 WHERE id = ?1",
            params![id_i64, status],
        )
        .map_err(internal)?
        > 0)
}

fn add_usage(conn: &Connection, day: &str, usage: Usage) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO usage(day, input_tokens, output_tokens) VALUES (?1, ?2, ?3) \
         ON CONFLICT(day) DO UPDATE SET input_tokens = input_tokens + excluded.input_tokens, \
         output_tokens = output_tokens + excluded.output_tokens",
        params![day, usage.input_tokens, usage.output_tokens],
    )
    .map_err(internal)?;
    Ok(())
}

fn read_usage(conn: &Connection, day: &str) -> Result<Usage, StoreError> {
    conn.query_row(
        "SELECT input_tokens, output_tokens FROM usage WHERE day = ?1",
        params![day],
        |row| {
            Ok(Usage {
                input_tokens: row.get(0)?,
                output_tokens: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(internal)?
    .map_or(Ok(Usage::default()), Ok)
}

fn create_web_session(
    conn: &Connection,
    token_hash: &[u8],
    user_id: &str,
    created_at: &str,
    expires_at: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO web_sessions(token_hash, user_id, created_at, expires_at, last_seen) \
         VALUES (?1, ?2, ?3, ?4, ?3)",
        params![token_hash, user_id, created_at, expires_at],
    )
    .map_err(internal)?;
    Ok(())
}

fn get_web_session(
    conn: &Connection,
    token_hash: &[u8],
    now: &str,
) -> Result<Option<WebSessionInfo>, StoreError> {
    conn.query_row(
        "SELECT user_id, created_at, expires_at FROM web_sessions \
         WHERE token_hash = ?1 AND expires_at > ?2",
        params![token_hash, now],
        |row| {
            Ok(WebSessionInfo {
                user_id: row.get(0)?,
                created_at: row.get(1)?,
                expires_at: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(internal)
}

fn touch_web_session(conn: &Connection, token_hash: &[u8], now: &str) -> Result<bool, StoreError> {
    Ok(conn
        .execute(
            "UPDATE web_sessions SET last_seen = ?2 WHERE token_hash = ?1",
            params![token_hash, now],
        )
        .map_err(internal)?
        > 0)
}

fn delete_web_session(conn: &Connection, token_hash: &[u8]) -> Result<bool, StoreError> {
    Ok(conn
        .execute(
            "DELETE FROM web_sessions WHERE token_hash = ?1",
            params![token_hash],
        )
        .map_err(internal)?
        > 0)
}

fn delete_all_web_sessions(conn: &Connection) -> Result<(), StoreError> {
    conn.execute("DELETE FROM web_sessions", [])
        .map_err(internal)?;
    Ok(())
}

fn create_session_row(
    conn: &Connection,
    channel: &str,
    chat_id: &str,
    user_id: &str,
    title: &str,
) -> Result<SessionId, StoreError> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO sessions (channel, chat_id, user_id, title, archived, summary, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 0, '', ?5, ?5)",
        params![channel, chat_id, user_id, title, now],
    )
    .map_err(internal)?;
    Ok(SessionId::new(conn.last_insert_rowid()))
}

fn ensure_session(
    conn: &Connection,
    channel: &str,
    chat_id: &str,
    user_id: &str,
    title: &str,
) -> Result<SessionId, StoreError> {
    let found = conn
        .query_row(
            "SELECT id FROM sessions WHERE channel = ?1 AND chat_id = ?2 AND archived = 0 \
             AND (?3 = '' OR user_id = '' OR user_id = ?3) ORDER BY id DESC LIMIT 1",
            params![channel, chat_id, user_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(internal)?;
    if let Some(id) = found {
        if !user_id.is_empty() {
            conn.execute(
                "UPDATE sessions SET user_id = ?2 \
                 WHERE id = ?1 AND (user_id = '' OR user_id = ?2)",
                params![id, user_id],
            )
            .map_err(internal)?;
        }
        return Ok(SessionId::new(id));
    }
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO sessions \
           (channel, chat_id, user_id, title, archived, summary, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 0, '', ?5, ?5)",
        params![channel, chat_id, user_id, title, now],
    )
    .map_err(internal)?;
    Ok(SessionId::new(conn.last_insert_rowid()))
}

fn archive_session(conn: &Connection, session: SessionId) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET archived = 1, updated_at = ?2 WHERE id = ?1",
        params![session.get(), now_rfc3339()],
    )
    .map_err(internal)?;
    Ok(())
}

/// Ghi message ngay khi phát sinh (agents.md mục 6): `seq` cấp trong **cùng** câu INSERT
/// nên không có khe hở tranh chấp.
fn append_message(
    conn: &Connection,
    session: SessionId,
    message: &Message,
) -> Result<i64, StoreError> {
    let content = serde_json::to_string(message)
        .map_err(|err| StoreError::Internal(format!("serialize message thất bại: {err}")))?;
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO messages (session_id, seq, role, content_json, text_for_search, created_at) \
         VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?1), \
         ?2, ?3, ?4, ?5)",
        params![
            session.get(),
            message.role.as_str(),
            content,
            message.text_for_search(),
            now
        ],
    )
    .map_err(internal)?;
    let message_id = conn.last_insert_rowid();
    // Tiêu đề lấy từ tin đầu tiên của người dùng (mục 8.1); chỉ đặt khi còn rỗng.
    conn.execute(
        "UPDATE sessions SET updated_at = ?2, \
         title = CASE WHEN title = '' AND ?3 <> '' THEN ?3 ELSE title END \
         WHERE id = ?1",
        params![session.get(), now, title_from(message)],
    )
    .map_err(internal)?;
    Ok(message_id)
}

fn count_messages(conn: &Connection, session: SessionId) -> Result<u64, StoreError> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session.get()],
            |row| row.get(0),
        )
        .map_err(internal)?;
    Ok(n.max(0) as u64)
}

fn clear_messages(conn: &Connection, session: SessionId) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM messages WHERE session_id = ?1",
        params![session.get()],
    )
    .map_err(internal)?;
    Ok(())
}

fn save_summary(conn: &Connection, session: SessionId, summary: &str) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET summary = ?2, updated_at = ?3 WHERE id = ?1",
        params![session.get(), summary, now_rfc3339()],
    )
    .map_err(internal)?;
    Ok(())
}

fn load_summary(conn: &Connection, session: SessionId) -> Result<Option<String>, StoreError> {
    let found = conn
        .query_row(
            "SELECT summary FROM sessions WHERE id = ?1",
            params![session.get()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(internal)?;
    Ok(found.filter(|text| !text.is_empty()))
}

fn memory_save_row(conn: &Connection, text: &str, tags: &str) -> Result<u64, StoreError> {
    conn.execute(
        "INSERT INTO memories (text, tags, created_at) VALUES (?1, ?2, ?3)",
        params![text, tags, now_rfc3339()],
    )
    .map_err(internal)?;
    Ok(conn.last_insert_rowid().max(0) as u64)
}

fn insert_outbound(
    conn: &Connection,
    channel: &str,
    chat_id: &str,
    payload: &Outbound,
    next_attempt_at: &str,
) -> Result<u64, StoreError> {
    let payload_json = serde_json::to_string(payload)
        .map_err(|err| StoreError::Internal(format!("serialize outbound thất bại: {err}")))?;
    conn.execute(
        "INSERT INTO outbox \
           (channel, chat_id, payload_json, attempts, created_at, next_attempt_at, last_error) \
         VALUES (?1, ?2, ?3, 0, ?4, ?5, '')",
        params![
            channel,
            chat_id,
            payload_json,
            now_rfc3339(),
            next_attempt_at
        ],
    )
    .map_err(internal)?;
    Ok(conn.last_insert_rowid().max(0) as u64)
}

fn load_due_outbox(
    conn: &Connection,
    now: &str,
    limit: usize,
) -> Result<Vec<OutboxEntry>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, channel, chat_id, payload_json, attempts, next_attempt_at, \
                    last_error, created_at FROM outbox WHERE next_attempt_at <= ?1 \
             ORDER BY created_at, id LIMIT ?2",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![now, sql_limit(limit)], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(internal)?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, channel, chat_id, payload, attempts, next_attempt_at, last_error, created_at) =
            row.map_err(internal)?;
        let id = u64::try_from(id)
            .map_err(|_| StoreError::Internal("outbox.id âm tính trong SQLite".into()))?;
        let attempts = u32::try_from(attempts)
            .map_err(|_| StoreError::Internal("outbox.attempts âm tính trong SQLite".into()))?;
        let payload = serde_json::from_str(&payload)
            .map_err(|err| StoreError::Internal(format!("outbox payload hỏng: {err}")))?;
        entries.push(OutboxEntry {
            id,
            channel,
            chat_id,
            payload,
            attempts,
            next_attempt_at,
            last_error,
            created_at,
        });
    }
    Ok(entries)
}

fn delete_outbox(conn: &Connection, id: u64) -> Result<(), StoreError> {
    let id = i64::try_from(id)
        .map_err(|_| StoreError::Internal("outbox.id vượt giới hạn SQLite".into()))?;
    conn.execute("DELETE FROM outbox WHERE id = ?1", params![id])
        .map_err(internal)?;
    Ok(())
}

fn update_outbox_retry(
    conn: &Connection,
    id: u64,
    next_attempt_at: &str,
    last_error: &str,
) -> Result<(), StoreError> {
    let id = i64::try_from(id)
        .map_err(|_| StoreError::Internal("outbox.id vượt giới hạn SQLite".into()))?;
    conn.execute(
        "UPDATE outbox SET attempts = attempts + 1, next_attempt_at = ?2, last_error = ?3 \
         WHERE id = ?1",
        params![id, next_attempt_at, truncate_chars(last_error, 1_000)],
    )
    .map_err(internal)?;
    Ok(())
}

fn load_history(
    conn: &Connection,
    session: SessionId,
    before_seq: Option<u64>,
    limit: usize,
) -> Result<Vec<Message>, StoreError> {
    let before = before_seq.map(|seq| seq as i64);
    let mut stmt = conn
        .prepare(
            "SELECT content_json FROM (\
               SELECT seq, content_json FROM messages \
               WHERE session_id = ?1 AND (?2 IS NULL OR seq < ?2) \
               ORDER BY seq DESC LIMIT ?3\
             ) ORDER BY seq ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session.get(), before, sql_limit(limit)], |row| {
            row.get::<_, String>(0)
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let json = row.map_err(internal)?;
        out.push(decode_message(&json)?);
    }
    Ok(out)
}

fn list_messages(
    conn: &Connection,
    session: SessionId,
    before_seq: Option<u64>,
    limit: usize,
) -> Result<Vec<StoredMessage>, StoreError> {
    let before = before_seq.map(|seq| seq as i64);
    let mut stmt = conn
        .prepare(
            "SELECT seq, content_json FROM (\
               SELECT seq, content_json FROM messages \
               WHERE session_id = ?1 AND (?2 IS NULL OR seq < ?2) \
               ORDER BY seq DESC LIMIT ?3\
             ) ORDER BY seq ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session.get(), before, sql_limit(limit)], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, json) = row.map_err(internal)?;
        out.push(StoredMessage {
            seq: seq.max(0) as u64,
            message: decode_message(&json)?,
        });
    }
    Ok(out)
}

fn delete_before(conn: &Connection, session: SessionId, before_seq: u64) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM messages WHERE session_id = ?1 AND seq < ?2",
        params![session.get(), before_seq as i64],
    )
    .map_err(internal)?;
    Ok(())
}

/// Tìm kiếm BM25 trong `memories` **và** `messages` (agents.md mục 8.4).
///
/// `MATCH` không bao giờ nhận query thô của model: mọi từ khoá được bọc trong dấu ngoặc
/// kép (`sanitize_fts_query`) nên cú pháp FTS không thể bị phá.
///
/// Điểm BM25 phụ thuộc **kích thước bảng** (bảng ít dòng ⇒ IDF ≈ 0 ⇒ điểm rất nhỏ), nên
/// điểm được chuẩn hoá **trong từng nguồn** trước khi trộn: `1.0` là kết quả liên quan
/// nhất của nguồn đó. Nhờ vậy một ghi nhớ dài hạn không bị "chìm" chỉ vì bảng `memories`
/// có ít dòng hơn `messages`.
fn memory_search(conn: &Connection, query: &str) -> Result<Vec<MemorySearchHit>, StoreError> {
    let Some(match_expr) = sanitize_fts_query(query) else {
        return Ok(Vec::new());
    };
    let limit = MEMORY_SEARCH_LIMIT as i64;
    let mut memories = search_memories(conn, &match_expr, limit)?;
    let mut messages = search_messages(conn, &match_expr, limit)?;
    normalize_scores(&mut memories);
    normalize_scores(&mut messages);

    // `sort_by` ổn định ⇒ hai kết quả cùng điểm (mỗi nguồn đều có điểm 1.0) giữ thứ tự
    // chèn: ghi nhớ dài hạn đứng trước, rồi tới lịch sử hội thoại.
    let mut hits = memories;
    hits.extend(messages);
    hits.sort_by(|left, right| right.score.total_cmp(&left.score));
    hits.truncate(MEMORY_SEARCH_LIMIT);
    Ok(hits)
}

/// Tìm trong bảng `memories` (chỉ mục FTS5 external content).
fn search_memories(
    conn: &Connection,
    match_expr: &str,
    limit: i64,
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.text, bm25(memories_fts) FROM memories_fts \
             JOIN memories m ON m.id = memories_fts.rowid \
             WHERE memories_fts MATCH ?1 ORDER BY 2 LIMIT ?2",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![match_expr, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (text, score) = row.map_err(internal)?;
        out.push(MemorySearchHit {
            score: bm25_to_score(score),
            text,
            source: MemorySource::Memories,
        });
    }
    Ok(out)
}

/// Tìm trong lịch sử hội thoại (`text_for_search` gồm cả tên/đối số tool).
fn search_messages(
    conn: &Connection,
    match_expr: &str,
    limit: i64,
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.text_for_search, m.session_id, bm25(messages_fts) \
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?1 ORDER BY 3 LIMIT ?2",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![match_expr, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (text, session_id, score) = row.map_err(internal)?;
        out.push(MemorySearchHit {
            score: bm25_to_score(score),
            text: truncate_chars(&text, MEMORY_HIT_PREVIEW_CHARS),
            source: MemorySource::Message { session_id },
        });
    }
    Ok(out)
}

/// Chia mọi điểm cho điểm cao nhất của **cùng nguồn** ⇒ miền giá trị `[0, 1]`.
fn normalize_scores(hits: &mut [MemorySearchHit]) {
    let best = hits.iter().map(|hit| hit.score).fold(0.0_f32, f32::max);
    if best > 0.0 {
        for hit in hits.iter_mut() {
            hit.score /= best;
        }
    }
}
/// `bm25()` trả giá trị **âm** (nhỏ hơn = liên quan hơn) ⇒ đảo dấu cho dễ đọc.
fn bm25_to_score(raw: f64) -> f32 {
    (-raw) as f32
}

/// Làm sạch query của người dùng/model trước khi đưa vào `MATCH` (agents.md mục 8.4).
///
/// Giữ lại ký tự chữ-số (kể cả dấu tiếng Việt) và `_`, bọc mỗi từ khoá trong `\"`.
/// Trả `None` khi không còn từ khoá nào — khi đó tìm kiếm trả về rỗng thay vì lỗi cú pháp.
fn sanitize_fts_query(query: &str) -> Option<String> {
    let mut tokens: Vec<String> = Vec::new();
    for raw in query.split_whitespace() {
        let cleaned: String = raw
            .chars()
            .filter(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        tokens.push(format!("\"{cleaned}\""));
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::PathBuf;

    use beanagent_llm::{FakeProvider, LlmProvider};
    use beanagent_types::{
        Config, LlmResponse, Message, Outbound, OutboundKind, Role, SessionId, ToolCall,
    };
    use tempfile::TempDir;

    use super::{
        MemorySearchHit, MemorySource, MemoryStore, SCHEMA_SQL, SqliteStore, Store, configure,
    };
    use crate::safe_cut::check_no_orphan_result;

    fn db_path(dir: &TempDir) -> PathBuf {
        dir.path().join("beanagent.db")
    }

    fn open(dir: &TempDir) -> SqliteStore {
        SqliteStore::open(&db_path(dir)).unwrap()
    }

    /// Một lượt "đọc file" hoàn chỉnh: user → assistant(gọi tool) → tool result.
    fn tool_round(index: usize, filler: &str) -> Vec<Message> {
        let id = format!("c{index}");
        vec![
            Message::user(format!("{filler} câu hỏi {index}")),
            Message::assistant(
                None,
                vec![ToolCall::new(
                    id.clone(),
                    "probe",
                    serde_json::json!({ "n": index }),
                )],
            ),
            Message::tool(id, format!("{filler} kết quả {index}")),
        ]
    }

    fn texts(hits: &[MemorySearchHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.text.as_str()).collect()
    }

    /// Bền vững qua khởi động lại (agents.md mục 20).
    #[tokio::test]
    async fn sqlite_persists_messages_across_restart() {
        let dir = TempDir::new().unwrap();
        let session;
        {
            let store = open(&dir);
            session = store.ensure_session("cli", "local", "").await.unwrap();
            store
                .append(session, Message::user("nhớ giúp tôi việc này"))
                .await
                .unwrap();
            store
                .append(session, Message::tool("c1", "kết quả"))
                .await
                .unwrap();
            store
                .memory_save("người dùng tên Vinh", "hồ sơ")
                .await
                .unwrap();
            store.save_summary(session, "tóm tắt cũ").await.unwrap();
        } // drop ⇒ worker dừng, connection đóng (WAL flush)

        let reopened = open(&dir);
        let session_again = reopened.ensure_session("cli", "local", "").await.unwrap();
        assert_eq!(
            session_again, session,
            "phiên phải được tái sử dụng sau restart"
        );
        let history = reopened.history(session, None, 0).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text.as_deref(), Some("nhớ giúp tôi việc này"));
        assert_eq!(history[1].role, Role::Tool);
        assert_eq!(history[1].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(
            reopened.summary(session).await.unwrap().as_deref(),
            Some("tóm tắt cũ")
        );
        let hits = reopened.memory_search("Vinh").await.unwrap();
        assert!(hits.iter().any(|h| h.source == MemorySource::Memories));
    }

    /// `/new`: lưu trữ phiên cũ (không xoá) rồi mở phiên mới cho cùng `chat_id`.
    #[tokio::test]
    async fn ensure_session_reuses_active_then_creates_new_after_archive() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let first = store.ensure_session("cli", "local", "").await.unwrap();
        assert_eq!(
            first,
            store.ensure_session("cli", "local", "").await.unwrap()
        );
        let other = store.ensure_session("cli", "khác", "").await.unwrap();
        assert_ne!(first, other);

        store.archive_session(first).await.unwrap();
        let next = store.ensure_session("cli", "local", "").await.unwrap();
        assert_ne!(next, first);
        store.append(next, Message::user("xin chào")).await.unwrap();
        assert_eq!(store.count(next).await.unwrap(), 1);
        assert_eq!(store.count(first).await.unwrap(), 0);
        assert_eq!(store.count(other).await.unwrap(), 0);
    }

    /// Outbox bền vững: ghi lỗi, tăng attempts, đổi lịch rồi xoá khi gửi thành công.
    #[tokio::test]
    async fn sqlite_outbox_retry_lifecycle_survives_restart() {
        let dir = TempDir::new().unwrap();
        let payload = Outbound {
            session_id: SessionId::new(7),
            message_id: 42,
            text: "tin chủ động 🦀".into(),
            kind: OutboundKind::Notification,
            action: None,
        };
        let id = {
            let store = open(&dir);
            let id = store
                .enqueue_outbound("telegram", "123", &payload, "2999-01-01T00:00:00.000000Z")
                .await
                .unwrap();
            let due = store
                .due_outbox("3000-01-01T00:00:00.000000Z", 10)
                .await
                .unwrap();
            assert_eq!(due.len(), 1);
            assert_eq!(due[0].payload, payload);
            store
                .retry_outbox(id, "2999-01-01T00:00:00.000000Z", "mạng lỗi: timeout")
                .await
                .unwrap();
            id
        };

        let reopened = open(&dir);
        let due = reopened
            .due_outbox("3000-01-01T00:00:00.000000Z", 10)
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
        assert_eq!(due[0].attempts, 1);
        assert_eq!(due[0].last_error, "mạng lỗi: timeout");
        reopened.complete_outbox(id).await.unwrap();
        assert!(
            reopened
                .due_outbox("3000-01-01T00:00:00.000000Z", 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// `user_id` gắn ownership session; user khác không nhận lại session cũ.
    #[tokio::test]
    async fn ensure_session_for_user_enforces_ownership() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let first = store
            .ensure_session_for_user("web", "chat-a", "web:admin", "")
            .await
            .unwrap();
        let info = store.session_info(first).await.unwrap().unwrap();
        assert_eq!(info.user_id, "web:admin");
        assert!(!info.archived);
        assert_eq!(
            first,
            store
                .ensure_session_for_user("web", "chat-a", "web:admin", "")
                .await
                .unwrap()
        );
        let other = store
            .ensure_session_for_user("web", "chat-a", "web:other", "")
            .await
            .unwrap();
        assert_ne!(other, first);
    }

    /// Migration v2 chịu được DB bị dừng giữa chừng sau khi đã thêm một cột.
    #[tokio::test]
    async fn migration_v2_resumes_after_partial_schema_change() {
        let dir = TempDir::new().unwrap();
        {
            let conn = rusqlite::Connection::open(db_path(&dir)).unwrap();
            configure(&conn).unwrap();
            conn.execute_batch(SCHEMA_SQL).unwrap();
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN user_id TEXT NOT NULL DEFAULT ''")
                .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }
        let store = open(&dir);
        let session = store
            .ensure_session_for_user("cli", "local", "cli:local", "")
            .await
            .unwrap();
        assert_eq!(
            store.session_info(session).await.unwrap().unwrap().user_id,
            "cli:local"
        );
        let version: i64 = rusqlite::Connection::open(db_path(&dir))
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    /// Tiêu đề phiên lấy từ **dòng đầu** của tin đầu tiên do người dùng gửi (mục 8.1).
    #[tokio::test]
    async fn session_title_comes_from_first_user_message() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        store
            .append(
                session,
                Message::user("Lập kế hoạch cho tuần này\nchi tiết ở dưới"),
            )
            .await
            .unwrap();
        store
            .append(session, Message::user("tin thứ hai"))
            .await
            .unwrap();
        let conn = rusqlite::Connection::open(db_path(&dir)).unwrap();
        let title: String = conn
            .query_row(
                "SELECT title FROM sessions WHERE id = ?1",
                rusqlite::params![session.get()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Lập kế hoạch cho tuần này");
    }

    /// `limit` giữ **message cuối**, `before_seq` phân trang ngược, `seq` bắt đầu từ 1.
    #[tokio::test]
    async fn history_honours_limit_and_paging() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        for index in 0..5 {
            store
                .append(session, Message::user(format!("tin {index}")))
                .await
                .unwrap();
        }
        assert_eq!(store.history(session, None, 0).await.unwrap().len(), 5);
        let last_two = store.history(session, None, 2).await.unwrap();
        assert_eq!(last_two.len(), 2);
        assert_eq!(last_two[0].text.as_deref(), Some("tin 3"));
        assert_eq!(last_two[1].text.as_deref(), Some("tin 4"));
        let page = store.history(session, Some(3), 2).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].text.as_deref(), Some("tin 0"));
        assert_eq!(page[1].text.as_deref(), Some("tin 1"));
        let stored = store.list_messages(session, None, 0).await.unwrap();
        assert_eq!(stored[0].seq, 1);
        assert_eq!(stored[4].seq, 5);
    }

    /// FTS5 tìm được cả `memories` lẫn `messages` (agents.md mục 8.4).
    #[tokio::test]
    async fn fts5_finds_memories_and_messages() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        store
            .memory_save("Người dùng thích báo cáo tài chính quý 4", "sở thích")
            .await
            .unwrap();
        store
            .append(session, Message::user("lập báo cáo tài chính quý 4"))
            .await
            .unwrap();
        store
            .append(session, Message::user("thời tiết hôm nay thế nào"))
            .await
            .unwrap();

        let hits = store.memory_search("báo cáo").await.unwrap();
        assert!(hits.iter().any(|h| h.source == MemorySource::Memories));
        assert!(
            hits.iter()
                .any(|h| matches!(h.source, MemorySource::Message { .. })),
            "phải tìm được trong lịch sử hội thoại: {:?}",
            texts(&hits)
        );
        assert!(hits.iter().all(|h| h.text.contains("báo")));
        assert!(store.memory_search("thời tiết").await.unwrap().len() == 1);
    }

    /// Điểm chuẩn hoá theo nguồn: ghi nhớ dài hạn không bị "chìm" chỉ vì bảng có ít dòng
    /// (BM25 của bảng 1 dòng có IDF ≈ 0).
    #[tokio::test]
    async fn fts5_normalizes_scores_per_source() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        store.memory_save("hồ sơ của Vinh", "hồ sơ").await.unwrap();
        store
            .append(session, Message::user("Vinh hỏi về hồ sơ"))
            .await
            .unwrap();

        let hits = store.memory_search("hồ sơ Vinh").await.unwrap();
        let best_memory = hits
            .iter()
            .find(|hit| hit.source == MemorySource::Memories)
            .unwrap();
        let best_message = hits
            .iter()
            .find(|hit| matches!(hit.source, MemorySource::Message { .. }))
            .unwrap();
        assert_eq!(best_memory.score, 1.0);
        assert_eq!(best_message.score, 1.0);
        // Bằng điểm ⇒ ghi nhớ dài hạn đứng trước (xem ghi chú trong `memory_search`).
        assert_eq!(hits[0].source, MemorySource::Memories);
    }

    /// BM25: term xuất hiện nhiều lần trong cùng độ dài văn bản phải xếp trước.
    #[tokio::test]
    async fn fts5_ranks_higher_term_frequency_first() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        store
            .memory_save("alpha beta gamma delta", "a")
            .await
            .unwrap();
        store
            .memory_save("alpha alpha alpha alpha alpha", "b")
            .await
            .unwrap();
        let hits = store.memory_search("alpha").await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].text, "alpha alpha alpha alpha alpha");
        assert!(hits[0].score > hits[1].score, "điểm: {:?}", hits);
    }

    /// Tiếng Việt: tìm có dấu và không dấu đều phải khớp (tokenizer `remove_diacritics 2`).
    #[tokio::test]
    async fn fts5_matches_vietnamese_with_or_without_diacritics() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        store
            .memory_save("Báo cáo tài chính quý 4 cần gửi trước thứ Sáu", "công việc")
            .await
            .unwrap();
        for query in ["báo cáo", "bao cao", "BÁO CÁO", "tai chinh", "thu sau"] {
            let hits = store.memory_search(query).await.unwrap();
            assert!(!hits.is_empty(), "không tìm thấy với query `{query}`");
        }
    }

    /// Query của model là đầu vào không tin cậy: mọi ký tự điều khiển FTS phải bị vô hiệu.
    #[tokio::test]
    async fn fts_search_tolerates_special_characters() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        store
            .memory_save("ghi nhớ về dự án BeanAgent", "test")
            .await
            .unwrap();
        for query in [
            "",
            "   ",
            "\"",
            "*",
            "( )",
            "NEAR(",
            "AND OR NOT",
            "^dự án$",
            "-",
            "dự án\" OR 1=1 --",
            "😀 emoji",
            "café:test*",
        ] {
            // Không được panic hay trả lỗi cú pháp FTS.
            store.memory_search(query).await.unwrap();
        }
        assert!(!store.memory_search("dự án").await.unwrap().is_empty());
        assert!(store.memory_search("😀").await.unwrap().is_empty());
    }

    /// Trigger FTS phải đồng bộ khi xoá (`delete_before`, `clear`).
    #[tokio::test]
    async fn delete_and_clear_keep_fts_in_sync() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        store
            .append(session, Message::user("chủ đề zebra"))
            .await
            .unwrap();
        store
            .append(session, Message::user("chủ đề unicorn"))
            .await
            .unwrap();
        assert!(!store.memory_search("zebra").await.unwrap().is_empty());

        store.delete_before(session, 2).await.unwrap();
        assert!(
            store.memory_search("zebra").await.unwrap().is_empty(),
            "FTS chưa đồng bộ sau khi xoá"
        );
        store.clear(session).await.unwrap();
        assert!(store.memory_search("unicorn").await.unwrap().is_empty());
        assert_eq!(store.count(session).await.unwrap(), 0);
    }

    /// Compaction (mục 8.3): tóm tắt phần cũ, giữ phần gần đây, **không** tách cặp tool.
    #[tokio::test]
    async fn compaction_keeps_tool_pairs_and_saves_summary() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        let filler = "dữ liệu khá dài để đo ngân sách token ".repeat(5);
        let filler = filler.as_str();
        for index in 0..12 {
            for message in tool_round(index, filler) {
                store.append(session, message).await.unwrap();
            }
        }
        assert_eq!(store.count(session).await.unwrap(), 36);

        let mut config = Config::default();
        config.agent.context_budget_tokens = 1_000;
        let provider = FakeProvider::new(vec![LlmResponse::text_only("TÓM TẮT: đang làm việc X")]);
        let llm: &dyn LlmProvider = &provider;
        store.compact(session, llm, &config).await.unwrap();

        assert_eq!(
            store.summary(session).await.unwrap().as_deref(),
            Some("TÓM TẮT: đang làm việc X")
        );
        // Giữ 20 message cuối ⇒ bắt đầu ở `user` của lượt thứ 7 (index 18).
        assert_eq!(store.count(session).await.unwrap(), 18);
        let history = store.history(session, None, 0).await.unwrap();
        assert_eq!(history.first().map(|m| m.role), Some(Role::User));
        assert!(
            check_no_orphan_result(&history, 0),
            "compaction đã tách cặp assistant/tool"
        );
        assert_eq!(
            history.last().unwrap().tool_call_id.as_deref(),
            Some("c11"),
            "phải giữ nguyên lượt gần nhất"
        );
        // Nội dung đã bị xoá không còn trong FTS.
        assert!(store.memory_search("kết quả 0").await.unwrap().is_empty());
    }

    /// Dưới ngưỡng 70% thì **không** gọi LLM và không đụng vào lịch sử.
    #[tokio::test]
    async fn compaction_skipped_when_under_budget() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir);
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        store
            .append(session, Message::user("một câu ngắn"))
            .await
            .unwrap();
        let config = Config::default();
        // Có sẵn câu trả lời ⇒ nếu bị gọi thì `summary` sẽ khác `None`.
        let provider = FakeProvider::new(vec![LlmResponse::text_only("không được gọi")]);
        let llm: &dyn LlmProvider = &provider;
        store.compact(session, llm, &config).await.unwrap();
        assert!(store.summary(session).await.unwrap().is_none());
        assert_eq!(store.count(session).await.unwrap(), 1);
    }

    /// Bản in-memory hành xử giống bản SQLite ở các thao tác cơ bản (M3 + M5).
    #[tokio::test]
    async fn memory_store_behaves_like_sqlite_for_basic_ops() {
        let store = MemoryStore::new();
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        assert_eq!(
            session,
            store.ensure_session("cli", "local", "").await.unwrap()
        );
        store.append(session, Message::user("một")).await.unwrap();
        store
            .append(session, Message::tool("c1", "hai"))
            .await
            .unwrap();
        assert_eq!(store.count(session).await.unwrap(), 2);
        let last = store.history(session, None, 1).await.unwrap();
        assert_eq!(last[0].text.as_deref(), Some("hai"));
        assert_eq!(
            store.list_messages(session, None, 0).await.unwrap()[1].seq,
            2
        );
        assert_eq!(
            store.memory_save("ghi nhớ về zeta", "tag").await.unwrap(),
            1
        );
        let hits = store.memory_search("zeta").await.unwrap();
        assert_eq!(hits[0].source, MemorySource::Memories);
        store.delete_before(session, 2).await.unwrap();
        assert_eq!(store.count(session).await.unwrap(), 1);
        store.archive_session(session).await.unwrap();
        assert_ne!(
            session,
            store.ensure_session("cli", "local", "").await.unwrap()
        );
        store.clear(session).await.unwrap();
        assert_eq!(store.count(session).await.unwrap(), 0);
        // `append` tự tạo phiên chưa biết (test M3 ghi thẳng `SessionId::new(99)`).
        store
            .append(SessionId::new(99), Message::user("trực tiếp"))
            .await
            .unwrap();
        assert_eq!(store.count(SessionId::new(99)).await.unwrap(), 1);
        assert!(store.summary(SessionId::new(99)).await.unwrap().is_none());
    }
}
