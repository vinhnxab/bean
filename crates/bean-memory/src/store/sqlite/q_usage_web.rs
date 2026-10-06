//! Truy van ngan sach token (`usage_day`) va phien dang nhap web (`web_sessions`).

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use crate::store::types::{StoreError, WebSessionInfo};
use bean_types::Usage;

pub(super) fn add_usage(conn: &Connection, day: &str, usage: Usage) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO usage(day, input_tokens, output_tokens) VALUES (?1, ?2, ?3) \
         ON CONFLICT(day) DO UPDATE SET input_tokens = input_tokens + excluded.input_tokens, \
         output_tokens = output_tokens + excluded.output_tokens",
        params![day, usage.input_tokens, usage.output_tokens],
    )
    .map_err(internal)?;
    Ok(())
}

pub(super) fn read_usage(conn: &Connection, day: &str) -> Result<Usage, StoreError> {
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

/// Cộng dồn usage của **một role** trong ngày (M21.7).
pub(super) fn add_usage_by_role(
    conn: &Connection,
    day: &str,
    role: &str,
    usage: Usage,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO usage_by_role(day, role, input_tokens, output_tokens) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(day, role) DO UPDATE SET \
           input_tokens = input_tokens + excluded.input_tokens, \
           output_tokens = output_tokens + excluded.output_tokens",
        params![day, role, usage.input_tokens, usage.output_tokens],
    )
    .map_err(internal)?;
    Ok(())
}

/// Đọc usage của một role; chưa có bản ghi ⇒ [`Usage::default`].
pub(super) fn read_usage_by_role(
    conn: &Connection,
    day: &str,
    role: &str,
) -> Result<Usage, StoreError> {
    conn.query_row(
        "SELECT input_tokens, output_tokens FROM usage_by_role WHERE day = ?1 AND role = ?2",
        params![day, role],
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

pub(super) fn create_web_session(
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

pub(super) fn get_web_session(
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

pub(super) fn touch_web_session(
    conn: &Connection,
    token_hash: &[u8],
    now: &str,
) -> Result<bool, StoreError> {
    Ok(conn
        .execute(
            "UPDATE web_sessions SET last_seen = ?2 WHERE token_hash = ?1",
            params![token_hash, now],
        )
        .map_err(internal)?
        > 0)
}

pub(super) fn delete_web_session(conn: &Connection, token_hash: &[u8]) -> Result<bool, StoreError> {
    Ok(conn
        .execute(
            "DELETE FROM web_sessions WHERE token_hash = ?1",
            params![token_hash],
        )
        .map_err(internal)?
        > 0)
}

pub(super) fn delete_all_web_sessions(conn: &Connection) -> Result<(), StoreError> {
    conn.execute("DELETE FROM web_sessions", [])
        .map_err(internal)?;
    Ok(())
}
