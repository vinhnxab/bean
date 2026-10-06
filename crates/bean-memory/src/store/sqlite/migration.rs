//! Pragma, migration và tiện ích dùng chung cho các truy vấn SQLite.
//!
//! # Vì sao tách riêng
//!
//! Migration theo `user_version` là **một chuỗi lịch sử** (mục 8.1) — đọc nó cần thấy
//! đủ các phiên bản trước, và khi debug "sai ở đâu" người ta cũng chỉ cần đọc file này.
//! Trước khi tách nó nằm lẫn trong khối hàm truy vấn, khiến việc tìm "phiên bản schema
//! hiện tại là bao nhiêu" phải lướt qua hàng trăm dòng SQL khác.

use std::time::Duration;

use rusqlite::Connection;

use super::schema::{PRAGMA_SQL, SCHEMA_SQL};
use crate::store::types::StoreError;
// ---------------------------------------------------------------------------
// Pragma, migration, tiện ích
// ---------------------------------------------------------------------------

pub(crate) fn configure(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(PRAGMA_SQL).map_err(internal)?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(internal)?;
    Ok(())
}

/// Migration tuần tự theo `user_version` (agents.md mục 8.1).
pub(super) fn run_migration(conn: &Connection) -> Result<(), StoreError> {
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
        version = 4;
    }
    if version < 5 {
        // M21.7: ngân sách token tách theo role. `IF NOT EXISTS` ⇒ chạy lại vô hại, kể cả
        // DB mới đã có bảng này trong `SCHEMA_SQL` (migration v1).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_by_role (
               day TEXT NOT NULL,
               role TEXT NOT NULL,
               input_tokens INTEGER NOT NULL DEFAULT 0,
               output_tokens INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY (day, role)
             );",
        )
        .map_err(|err| {
            StoreError::Internal(format!("migration v5 usage_by_role thất bại: {err}"))
        })?;
        conn.pragma_update(None, "user_version", 5)
            .map_err(internal)?;
        version = 5;
    }
    if version < 6 {
        // M25: token client MCP (chỉ lưu hash). `IF NOT EXISTS` ⇒ chạy lại vô hại, kể cả
        // DB mới đã có bảng này trong `SCHEMA_SQL` (migration v1).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS mcp_clients (
               token_hash TEXT PRIMARY KEY,
               name TEXT NOT NULL UNIQUE,
               role TEXT NOT NULL,
               created_at TEXT NOT NULL,
               expires_at TEXT NOT NULL DEFAULT ''
             );",
        )
        .map_err(|err| StoreError::Internal(format!("migration v6 mcp_clients thất bại: {err}")))?;
        conn.pragma_update(None, "user_version", 6)
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
pub(super) fn internal(err: rusqlite::Error) -> StoreError {
    StoreError::Internal(format!("SQLite: {err}"))
}
