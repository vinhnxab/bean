//! Truy van phien: doc/ghi `sessions`, tim phien active theo `(channel, chat_id, user_id)`.
//!
//! Nhom nay gom ca phien, message record va bo nho (`memories`) vi chung bang `memories`/`messages`.

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use crate::store::shared::{now_rfc3339, sql_limit};
use crate::store::types::{MemoryRecord, MessageRecord, SessionInfo, SessionSummary, StoreError};
use bean_types::{Message, SessionId};

pub(super) fn decode_message(json: &str) -> Result<Message, StoreError> {
    serde_json::from_str(json)
        .map_err(|err| StoreError::Internal(format!("message trong DB không đọc được: {err}")))
}

pub(super) fn load_session_info(
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

pub(super) fn load_session_summary(
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

pub(super) fn update_session(
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

pub(super) fn delete_session(
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

pub(super) fn list_sessions(
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

pub(super) fn message_by_id(
    conn: &Connection,
    id: i64,
) -> Result<Option<MessageRecord>, StoreError> {
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

pub(super) fn list_message_records(
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

pub(super) fn list_memories(
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

pub(super) fn delete_memory(conn: &Connection, id: u64) -> Result<bool, StoreError> {
    let id =
        i64::try_from(id).map_err(|_| StoreError::Internal("memory id vượt giới hạn".into()))?;
    Ok(conn
        .execute("DELETE FROM memories WHERE id = ?1", params![id])
        .map_err(internal)?
        > 0)
}

pub(super) fn find_active_session(
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
