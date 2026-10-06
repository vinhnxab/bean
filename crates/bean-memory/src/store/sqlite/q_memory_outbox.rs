//! Ghi nho dai han (`memories`) va hang doi gui lai (`outbox`) — muc 8 va 10.

use rusqlite::{Connection, params};

use super::migration::internal;
use crate::store::shared::now_rfc3339;
use crate::store::shared::{sql_limit, truncate_chars};
use crate::store::types::{OutboxEntry, StoreError};
use bean_types::Outbound;

pub(super) fn memory_save_row(
    conn: &Connection,
    text: &str,
    tags: &str,
) -> Result<u64, StoreError> {
    conn.execute(
        "INSERT INTO memories (text, tags, created_at) VALUES (?1, ?2, ?3)",
        params![text, tags, now_rfc3339()],
    )
    .map_err(internal)?;
    Ok(conn.last_insert_rowid().max(0) as u64)
}

pub(super) fn insert_outbound(
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

pub(super) fn load_due_outbox(
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

pub(super) fn delete_outbox(conn: &Connection, id: u64) -> Result<(), StoreError> {
    let id = i64::try_from(id)
        .map_err(|_| StoreError::Internal("outbox.id vượt giới hạn SQLite".into()))?;
    conn.execute("DELETE FROM outbox WHERE id = ?1", params![id])
        .map_err(internal)?;
    Ok(())
}

pub(super) fn update_outbox_retry(
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
