//! Truy van tac vu dinh ky (`scheduled_tasks`) — nguon cua scheduler (muc 14).

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use crate::store::shared::now_rfc3339;
use crate::store::shared::sql_limit;
use crate::store::types::{NewScheduledTask, ScheduledTask, StoreError};
use bean_types::SessionId;

pub(super) fn task_select(where_clause: &str) -> String {
    format!(
        "SELECT id, cron, prompt, session_id, channel, chat_id, allowed_tools, next_run, \
                enabled, created_at, last_run_at, last_status
         FROM scheduled_tasks {where_clause}"
    )
}

pub(super) fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduledTask> {
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

pub(super) fn list_tasks(conn: &Connection) -> Result<Vec<ScheduledTask>, StoreError> {
    let mut stmt = conn
        .prepare(&task_select("ORDER BY id DESC"))
        .map_err(internal)?;
    let rows = stmt.query_map([], task_from_row).map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

pub(super) fn list_tasks_for_session(
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

pub(super) fn create_task(
    conn: &Connection,
    task: NewScheduledTask,
) -> Result<ScheduledTask, StoreError> {
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

pub(super) fn load_task(conn: &Connection, id: u64) -> Result<Option<ScheduledTask>, StoreError> {
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

pub(super) fn set_task_enabled(
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

pub(super) fn delete_task_for_session(
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

pub(super) fn delete_task(conn: &Connection, id: u64) -> Result<bool, StoreError> {
    let id_i64 = i64::try_from(id)
        .map_err(|_| StoreError::Internal("scheduled task id vượt giới hạn SQLite".into()))?;
    Ok(conn
        .execute("DELETE FROM scheduled_tasks WHERE id = ?1", params![id_i64])
        .map_err(internal)?
        > 0)
}

pub(super) fn due_tasks(
    conn: &Connection,
    now: &str,
    limit: usize,
) -> Result<Vec<ScheduledTask>, StoreError> {
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

pub(super) fn claim_task(
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

pub(super) fn set_task_status(
    conn: &Connection,
    id: u64,
    status: &str,
) -> Result<bool, StoreError> {
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
