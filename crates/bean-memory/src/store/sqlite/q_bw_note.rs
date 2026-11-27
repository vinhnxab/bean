//! Truy vấn Bean World — nhóm **note**: ghi chú trao đổi append-only trên task.
//! Thêm note phải kiểm tra task tồn tại (trả lỗi đọc được, không để FK từ chối 500).

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use super::q_bw::{NOTES_MAX_CHARS, id_to_sql};
use crate::store::shared::now_rfc3339;
use crate::store::types::{NewWorkNote, StoreError, WorkNote};

fn note_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkNote> {
    Ok(WorkNote {
        id: row.get::<_, i64>(0)?.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                "note id âm".into(),
            )
        })?,
        task_id: row.get::<_, i64>(1)?.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Integer,
                "task id âm".into(),
            )
        })?,
        author: row.get(2)?,
        text: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) fn add_work_note(conn: &Connection, note: NewWorkNote) -> Result<WorkNote, StoreError> {
    let text = note.text.trim();
    if text.is_empty() || text.chars().count() > NOTES_MAX_CHARS {
        return Err(StoreError::Internal(format!(
            "ghi chú phải 1..={NOTES_MAX_CHARS} ký tự"
        )));
    }
    let task_id_i64 = id_to_sql(note.task_id)?;
    let task_exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM work_tasks WHERE id = ?1",
            params![task_id_i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(internal)?;
    if task_exists.is_none() {
        return Err(StoreError::Internal(format!(
            "task id={} không tồn tại",
            note.task_id
        )));
    }
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO work_notes (task_id, author, text, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![task_id_i64, note.author, text, now],
    )
    .map_err(|err| StoreError::Internal(format!("thêm ghi chú thất bại: {err}")))?;
    let id = conn.last_insert_rowid();
    // Task có hoạt động mới: đẩy updated_at để board xếp lại theo sửa đổi.
    conn.execute(
        "UPDATE work_tasks SET updated_at = ?1 WHERE id = ?2",
        params![now, task_id_i64],
    )
    .map_err(internal)?;
    Ok(WorkNote {
        id: id
            .try_into()
            .map_err(|_| StoreError::Internal("work_notes.last_insert_rowid âm".into()))?,
        task_id: note.task_id,
        author: note.author,
        text: text.to_string(),
        created_at: now,
    })
}

pub(super) fn list_work_notes(
    conn: &Connection,
    task_id: u64,
) -> Result<Vec<WorkNote>, StoreError> {
    let task_id_i64 = id_to_sql(task_id)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, task_id, author, text, created_at FROM work_notes
             WHERE task_id = ?1 ORDER BY id ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![task_id_i64], note_from_row)
        .map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}
