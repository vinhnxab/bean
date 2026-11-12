//! Kênh lệnh giữa async và worker thread: [`DbCommand`] + [`Reply`].
//!
//! # Vì sao đây là file riêng — và vì sao nó là **ranh giới quan trọng nhất** của thư mục
//!
//! `rusqlite` là API blocking (agents.md mục 22.8) nên mọi truy cập đi qua **một**
//! thread giữ connection. Mỗi method của [`Store`](crate::store::Store) tương ứng
//! **đúng một** biến thể của [`DbCommand`].
//!
//! Nhờ vậy ranh giới sync ⇄ async là **có kiểm soát**: thêm một method mới mà quên
//! thêm biến thể sẽ **không biên dịch được**, thay vì âm thầm chạy SQL trên runtime.
//!
//! Đổi tên một biến thể ở đây sẽ làm hỏng cả `mod.rs` (nơi đóng gói lệnh) và
//! `worker.rs` (nơi thực thi) — đó là điều tốt: đây là tập hợp thay đổi nhỏ nhất mà
//! chạm vào đúng những chỗ phải đồng bộ.

use tokio::sync::oneshot;

use bean_types::{Message, Outbound, SessionId, Usage};

use crate::store::types::{
    McpClientInfo, MemoryRecord, MemorySearchHit, MessageRecord, NewScheduledTask, OutboxEntry,
    ScheduledTask, SessionInfo, SessionSummary, StoreError, StoredMessage, WebSessionInfo,
};

/// Kênh trả lời của một lệnh gửi cho worker.
pub(super) type Reply<T> = oneshot::Sender<Result<T, StoreError>>;

/// Phân loại lệnh để định tuyến tới worker đúng (K4).
///
/// Lệnh **đọc** chạy trên connection reader riêng: WAL chịu nhiều reader song
/// song với writer, nên truy vấn FTS nặng (`memory_search`, `list_sessions`)
/// không còn chặn `append` — ghi message phải ngay khi phát sinh (mục 6).
/// Lệnh **ghi** chạy trên worker giữ connection ghi. [`DbCommand::kind`] là
/// `match` vét cạn: thêm biến thể mới mà chưa phân loại sẽ **không biên dịch**
/// được; phân loại sai chiều ghi→đọc thì lộ ngay vì reader bật `query_only`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CmdKind {
    Read,
    Write,
}

impl DbCommand {
    /// Lệnh này phải chạy trên reader hay writer?
    pub(super) fn kind(&self) -> CmdKind {
        match self {
            DbCommand::History { .. }
            | DbCommand::Count { .. }
            | DbCommand::SessionInfo { .. }
            | DbCommand::Summary { .. }
            | DbCommand::SessionSummary { .. }
            | DbCommand::FindActiveSession { .. }
            | DbCommand::ListSessions { .. }
            | DbCommand::MessageById { .. }
            | DbCommand::ListMessageRecords { .. }
            | DbCommand::ListMemories { .. }
            | DbCommand::ListTasks { .. }
            | DbCommand::ListTasksForSession { .. }
            | DbCommand::DueTasks { .. }
            | DbCommand::Usage { .. }
            | DbCommand::UsageByRole { .. }
            | DbCommand::GetWebSession { .. }
            | DbCommand::GetMcpClient { .. }
            | DbCommand::ListMcpClients { .. }
            | DbCommand::ToolOverrides { .. }
            | DbCommand::MemorySearch { .. }
            | DbCommand::DueOutbox { .. }
            | DbCommand::ListMessages { .. } => CmdKind::Read,
            DbCommand::EnsureSession { .. }
            | DbCommand::CreateSession { .. }
            | DbCommand::ArchiveSession { .. }
            | DbCommand::Append { .. }
            | DbCommand::Clear { .. }
            | DbCommand::SaveSummary { .. }
            | DbCommand::UpdateSession { .. }
            | DbCommand::DeleteSession { .. }
            | DbCommand::DeleteMemory { .. }
            | DbCommand::CreateTask { .. }
            | DbCommand::SetTaskEnabled { .. }
            | DbCommand::DeleteTaskForSession { .. }
            | DbCommand::DeleteTask { .. }
            | DbCommand::ClaimTask { .. }
            | DbCommand::SetTaskStatus { .. }
            | DbCommand::AddUsage { .. }
            | DbCommand::AddUsageByRole { .. }
            | DbCommand::CreateWebSession { .. }
            | DbCommand::TouchWebSession { .. }
            | DbCommand::DeleteWebSession { .. }
            | DbCommand::DeleteAllWebSessions { .. }
            | DbCommand::CreateMcpClient { .. }
            | DbCommand::DeleteMcpClient { .. }
            | DbCommand::SetToolEnabled { .. }
            | DbCommand::MemorySave { .. }
            | DbCommand::Outbound { .. }
            | DbCommand::CompleteOutbox { .. }
            | DbCommand::RetryOutbox { .. }
            | DbCommand::DeleteBefore { .. } => CmdKind::Write,
        }
    }
}

/// Lệnh gửi cho worker thread.
///
/// Mọi truy cập SQLite đi qua thread worker (K4: hai thread — writer giữ
/// connection ghi, reader giữ connection chỉ đọc; agents.md mục 22.8):
/// `rusqlite` là API blocking nên không được gọi trực tiếp trong async.
#[derive(Debug)]
pub(super) enum DbCommand {
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
    AddUsageByRole {
        day: String,
        role: String,
        usage: Usage,
        reply: Reply<()>,
    },
    UsageByRole {
        day: String,
        role: String,
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
    // (M25) Token client MCP: chỉ lưu hash, tách khỏi `web_sessions` vì TTL và
    // vòng đời khác nhau (token MCP dài hạn, không phải phiên đăng nhập).
    CreateMcpClient {
        token_hash: String,
        name: String,
        role: String,
        created_at: String,
        expires_at: String,
        reply: Reply<()>,
    },
    GetMcpClient {
        token_hash: String,
        now: String,
        reply: Reply<Option<McpClientInfo>>,
    },
    ListMcpClients {
        reply: Reply<Vec<McpClientInfo>>,
    },
    DeleteMcpClient {
        name: String,
        reply: Reply<bool>,
    },
    // (Runtime toggle) Bật/tắt tool từ UI: hàng tồn tại = tool bị tắt.
    SetToolEnabled {
        name: String,
        enabled: bool,
        updated_by: String,
        reply: Reply<()>,
    },
    ToolOverrides {
        reply: Reply<Vec<crate::traits::traits::ToolOverride>>,
    },
    MemorySave {
        text: String,
        tags: String,
        reply: Reply<u64>,
    },
    MemorySearch {
        query: String,
        exclude_current_run_of: Option<SessionId>,
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
