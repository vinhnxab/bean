//! Bộ lưu trữ lịch sử hội thoại (agents.md mục 8).
//!
//! M3: bản **in-memory** (T), đủ cho test và CLI. M5 thay bằng SQLite + FTS5.

use std::collections::HashMap;
use std::sync::Arc;

use beanagent_types::{Message, SessionId};
use tokio::sync::RwLock;

/// Giao diện lưu trữ lịch sử: ghi/đọc message, quản lý session.
#[async_trait::async_trait]
pub trait Store: Send + Sync {
    /// Ghi một message vào phiên. `seq` do store tự quản lý (tăng dần).
    async fn append(&self, session: SessionId, msg: Message) -> Result<(), StoreError>;

    /// Lấy lịch sử message của phiên, từ `before_seq` trở về trước (mặc định: tất cả).
    async fn history(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Message>, StoreError>;

    /// Lấy số lượng message hiện có của phiên (dùng để ước lượng token).
    async fn count(&self, session: SessionId) -> Result<u64, StoreError>;

    /// Xoá toàn bộ message của phiên (dùng cho `/new`).
    async fn clear(&self, session: SessionId) -> Result<(), StoreError>;
}

/// Lỗi store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("phiên không tồn tại: {0}")]
    NotFound(SessionId),
    #[error("lỗi nội bộ store: {0}")]
    Internal(String),
}

/// Bộ lưu trữ in-memory dùng `RwLock<HashMap>`. Chỉ an toàn cho **một tiến trình**,
/// không phù hợp cho production — M5 sẽ thay bằng SQLite.
#[derive(Default)]
pub struct MemoryStore {
    sessions: Arc<RwLock<HashMap<SessionId, Vec<Message>>>>,
}

impl MemoryStore {
    /// Tạo store trống.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Store for MemoryStore {
    async fn append(&self, session: SessionId, msg: Message) -> Result<(), StoreError> {
        let mut map = self.sessions.write().await;
        map.entry(session).or_default().push(msg);
        Ok(())
    }

    async fn history(
        &self,
        session: SessionId,
        before_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Message>, StoreError> {
        let map = self.sessions.read().await;
        let msgs = map.get(&session).ok_or(StoreError::NotFound(session))?;
        // `before_seq`: chỉ lấy các message **trước** seq này; trả về theo thứ tự thời gian
        // (cũ → mới), tối đa `limit` message gần nhất (`limit = 0` ⇒ không giới hạn).
        let upper = match before_seq {
            Some(seq) => (seq as usize).min(msgs.len()),
            None => msgs.len(),
        };
        let start = if limit == 0 {
            0
        } else {
            upper.saturating_sub(limit)
        };
        Ok(msgs[start..upper].to_vec())
    }

    async fn count(&self, session: SessionId) -> Result<u64, StoreError> {
        let map = self.sessions.read().await;
        Ok(map.get(&session).map_or(0, |v| v.len() as u64))
    }

    async fn clear(&self, session: SessionId) -> Result<(), StoreError> {
        let mut map = self.sessions.write().await;
        map.remove(&session);
        Ok(())
    }
}
