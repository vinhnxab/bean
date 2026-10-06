//! Ghi/doc message va summary cua phien (`messages`) — phan loi cua lich su hoi thoai.

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use crate::store::shared::now_rfc3339;
use crate::store::shared::title_from;
use crate::store::types::StoreError;
use bean_types::{Message, SessionId};

pub(super) fn create_session_row(
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

pub(super) fn ensure_session(
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

pub(super) fn archive_session(conn: &Connection, session: SessionId) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET archived = 1, updated_at = ?2 WHERE id = ?1",
        params![session.get(), now_rfc3339()],
    )
    .map_err(internal)?;
    Ok(())
}

/// Ghi message ngay khi phát sinh (agents.md mục 6): `seq` cấp trong **cùng** câu INSERT
/// nên không có khe hở tranh chấp.
pub(super) fn append_message(
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

pub(super) fn count_messages(conn: &Connection, session: SessionId) -> Result<u64, StoreError> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session.get()],
            |row| row.get(0),
        )
        .map_err(internal)?;
    Ok(n.max(0) as u64)
}

pub(super) fn clear_messages(conn: &Connection, session: SessionId) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM messages WHERE session_id = ?1",
        params![session.get()],
    )
    .map_err(internal)?;
    Ok(())
}

pub(super) fn save_summary(
    conn: &Connection,
    session: SessionId,
    summary: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET summary = ?2, updated_at = ?3 WHERE id = ?1",
        params![session.get(), summary, now_rfc3339()],
    )
    .map_err(internal)?;
    Ok(())
}

pub(super) fn load_summary(
    conn: &Connection,
    session: SessionId,
) -> Result<Option<String>, StoreError> {
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
