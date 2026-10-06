//! [`MemoryStore`] — bản cài đặt in-memory (M3), dùng cho test và demo.
//!
//! # Vì sao tách khỏi [`super`]
//!
//! Đây là **test double hợp lệ**, không phải code chết: nó cho phép test vòng lặp agent
//! chạy không cần file SQLite (nhanh, không đụng đĩa, chạy song song an toàn). Giữ nó
//! tách riêng để người đọc `sqlite/` không phải lọc qua 900 dòng không liên quan, và để
//! khi tối ưu hoá SQLite thì chỗ cần đọc không bị chôn vùi.
//!
//! # Bất biến
//!
//! * **Cùng ngữ nghĩa với SQLite** cho mọi method của [`Store`](super::Store). Hai bản
//!   có test dùng chung (`tests.rs`) chính là để giữ ràng buộc này — đổi hành vi ở
//!   một bản mà quên bản kia sẽ làm test đỏ.
//! * Không `unwrap`/`expect` (agents.md mục 0.8): lỗi trả về qua
//!   [`StoreError`](super::StoreError).
// ---------------------------------------------------------------------------
// Bản in-memory (M3)
// ---------------------------------------------------------------------------

use bean_llm::LlmProvider;
use bean_types::{Config, Message, Outbound, SessionId, Usage};
use std::collections::BTreeMap;
use tokio::sync::RwLock;

use super::compaction::compact_via;
use super::shared::{now_rfc3339, seq_to_index, slice_start, title_from, truncate_chars};
use super::types::{
    McpClientInfo, MemoryRecord, MemorySearchHit, MemorySource, MessageRecord, NewScheduledTask,
    OutboxEntry, ScheduledTask, SessionInfo, SessionSummary, StoreError, StoredMessage,
    WebSessionInfo,
};
use super::{MEMORY_HIT_PREVIEW_CHARS, MEMORY_SEARCH_LIMIT, Store};
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
/// (`bean-core/tests/agent_loop.rs`) và demo không cần đụng đĩa.
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
    usage_by_role: RwLock<BTreeMap<(String, String), Usage>>,
    web_sessions: RwLock<Vec<(Vec<u8>, WebSessionInfo)>>,
    mcp_clients: RwLock<Vec<(String, McpClientInfo)>>,
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
            usage_by_role: RwLock::new(BTreeMap::new()),
            web_sessions: RwLock::new(Vec::new()),
            mcp_clients: RwLock::new(Vec::new()),
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

    async fn add_usage_by_role(
        &self,
        day: &str,
        role: &str,
        usage: Usage,
    ) -> Result<(), StoreError> {
        let mut values = self.usage_by_role.write().await;
        let current = values
            .entry((day.to_string(), role.to_string()))
            .or_default();
        current.input_tokens = current.input_tokens.saturating_add(usage.input_tokens);
        current.output_tokens = current.output_tokens.saturating_add(usage.output_tokens);
        Ok(())
    }

    async fn usage_by_role(&self, day: &str, role: &str) -> Result<Usage, StoreError> {
        Ok(self
            .usage_by_role
            .read()
            .await
            .get(&(day.to_string(), role.to_string()))
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

    /// (M25) Cấp token client MCP — chỉ lưu hash, giống `web_sessions`.
    ///
    /// Cấp lại cho cùng một tên ⇒ xoá bản ghi cũ trước, nên không tồn tại hai token
    /// cùng lúc cho một client (tránh việc thu hồi "token mới" nhưng token cũ vẫn sống).
    async fn create_mcp_client(
        &self,
        token_hash: &str,
        name: &str,
        role: &str,
        created_at: &str,
        expires_at: &str,
    ) -> Result<(), StoreError> {
        let info = McpClientInfo {
            name: name.to_string(),
            role: role.to_string(),
            created_at: created_at.to_string(),
            expires_at: expires_at.to_string(),
        };
        let mut clients = self.mcp_clients.write().await;
        // Xoá theo **tên** (khớp SQL `DELETE ... WHERE name = ?1`): cấp lại cho cùng một
        // client phải làm token cũ mất hiệu lực, nếu không sẽ tồn tại hai token cùng
        // lúc và `revoke` chỉ xoá được một. Đây đúng là loại lệch hành vi K7 cảnh báo.
        clients.retain(|(hash, info)| hash != token_hash && info.name != name);
        clients.push((token_hash.to_string(), info));
        Ok(())
    }

    /// (M25) `expires_at` rỗng ⇒ không hết hạn; ngược lại phải còn hạn tới `now`.
    async fn get_mcp_client(
        &self,
        token_hash: &str,
        now: &str,
    ) -> Result<Option<McpClientInfo>, StoreError> {
        Ok(self
            .mcp_clients
            .read()
            .await
            .iter()
            .find(|(hash, info)| {
                hash == token_hash && (info.expires_at.is_empty() || info.expires_at.as_str() > now)
            })
            .map(|(_, info)| info.clone()))
    }

    /// (M25) Sắp xếp theo thời điểm cấp rồi tên để `list` có thứ tự ổn định.
    async fn list_mcp_clients(&self) -> Result<Vec<McpClientInfo>, StoreError> {
        let mut list: Vec<McpClientInfo> = self
            .mcp_clients
            .read()
            .await
            .iter()
            .map(|(_, info)| info.clone())
            .collect();
        list.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(list)
    }

    /// (M25) Thu hồi token của client theo tên.
    async fn delete_mcp_client(&self, name: &str) -> Result<bool, StoreError> {
        let mut clients = self.mcp_clients.write().await;
        let before = clients.len();
        clients.retain(|(_, info)| info.name != name);
        Ok(clients.len() != before)
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
