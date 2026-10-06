//! `impl Store for SqliteStore` — 51 method, mỗi method chỉ đóng gói lệnh.
//!
//! # Vì sao để nguyên một khối lớn
//!
//! Đây là **bảng tra cứu**, không phải logic: đọc nó để hiểu "method này đóng gói lệnh
//! nào" là hữu ích, nhưng tách nó ra nhiều file sẽ **làm giảm** khả năng kiểm chứng —
//! không ai đọc rời 8 file để đối chiếu xem có đủ 51 method không.
//!
//! Phần thật (SQL, decode, migration) nằm ở `q_*.rs` và `migration.rs`; file này cố
//! tình không chứa SQL nào để ranh giới "chỉ đóng gói" luôn nhìn thấy được.

use bean_llm::LlmProvider;
use bean_types::{Config, Message, Outbound, SessionId, Usage};

use super::command::DbCommand;
use super::inner::SqliteStore;
use crate::store::Store;
use crate::store::compaction::compact_via;
use crate::store::types::{
    McpClientInfo, MemoryRecord, MemorySearchHit, MessageRecord, NewScheduledTask, OutboxEntry,
    ScheduledTask, SessionInfo, SessionSummary, StoreError, StoredMessage, WebSessionInfo,
};
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

    async fn add_usage_by_role(
        &self,
        day: &str,
        role: &str,
        usage: Usage,
    ) -> Result<(), StoreError> {
        let day = day.to_string();
        let role = role.to_string();
        self.request(move |reply| DbCommand::AddUsageByRole {
            day,
            role,
            usage,
            reply,
        })
        .await
    }

    async fn usage_by_role(&self, day: &str, role: &str) -> Result<Usage, StoreError> {
        let day = day.to_string();
        let role = role.to_string();
        self.request(move |reply| DbCommand::UsageByRole { day, role, reply })
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

    async fn create_mcp_client(
        &self,
        token_hash: &str,
        name: &str,
        role: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), StoreError> {
        let token_hash = token_hash.to_string();
        let name = name.to_string();
        let role = role.to_string();
        let created_at = created_at.to_string();
        let expires_at = expires_at.to_string();
        self.request(move |reply| DbCommand::CreateMcpClient {
            token_hash,
            name,
            role,
            created_at,
            expires_at,
            reply,
        })
        .await
    }

    async fn get_mcp_client(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<Option<McpClientInfo>, StoreError> {
        let token_hash = token_hash.to_string();
        let now = now.to_string();
        self.request(move |reply| DbCommand::GetMcpClient {
            token_hash,
            now,
            reply,
        })
        .await
    }

    async fn list_mcp_clients(&self) -> Result<Vec<McpClientInfo>, StoreError> {
        self.request(|reply| DbCommand::ListMcpClients { reply })
            .await
    }

    async fn delete_mcp_client(&self, name: &str) -> Result<bool, StoreError> {
        let name = name.to_string();
        self.request(move |reply| DbCommand::DeleteMcpClient { name, reply })
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
