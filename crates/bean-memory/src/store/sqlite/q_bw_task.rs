//! Truy vấn Bean World — nhóm **task**: lọc/liệt kê, tạo, cập nhật một phần,
//! xoá và danh sách việc chưa xong (nguồn cho inject vào system prompt).
//! Mọi giá trị đi qua placeholder `?N`; chỉ nối tên cột cố định vào SQL.

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use super::migration::internal;
use super::q_bw::{
    NOTES_MAX_CHARS, TASK_TITLE_MAX_CHARS, id_to_sql, validate_priority, validate_status,
};
use crate::store::shared::now_rfc3339;
use crate::store::types::{NewWorkTask, StoreError, WorkTask, WorkTaskFilter, WorkTaskUpdate};

const TASK_COLUMNS: &str = "id, project_id, title, notes, status, priority, assignee, \
     created_by, created_at, updated_at, done_at";

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkTask> {
    Ok(WorkTask {
        id: row.get::<_, i64>(0)?.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                "task id âm".into(),
            )
        })?,
        project_id: row.get::<_, i64>(1)?.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Integer,
                "project id âm".into(),
            )
        })?,
        title: row.get(2)?,
        notes: row.get(3)?,
        status: row.get(4)?,
        priority: row.get(5)?,
        assignee: row.get(6)?,
        created_by: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        done_at: row.get(10)?,
    })
}

pub(super) fn list_work_tasks(
    conn: &Connection,
    filter: &WorkTaskFilter,
) -> Result<Vec<WorkTask>, StoreError> {
    // Chỉ nối **tên cột cố định**; mọi giá trị đi qua placeholder `?N` (an toàn injection).
    let mut conditions: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(project_id) = filter.project_id {
        args.push(Box::new(id_to_sql(project_id)?));
        conditions.push(format!("project_id = ?{}", args.len()));
    }
    if let Some(status) = &filter.status {
        validate_status(status)?;
        args.push(Box::new(status.clone()));
        conditions.push(format!("status = ?{}", args.len()));
    }
    if let Some(assignee) = &filter.assignee {
        args.push(Box::new(assignee.clone()));
        conditions.push(format!("assignee = ?{}", args.len()));
    }
    let mut sql = format!("SELECT {TASK_COLUMNS} FROM work_tasks");
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC");
    let mut stmt = conn.prepare(&sql).map_err(internal)?;
    let rows = stmt
        .query_map(params_from_iter(args), task_from_row)
        .map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

pub(super) fn create_work_task(
    conn: &Connection,
    task: NewWorkTask,
) -> Result<WorkTask, StoreError> {
    let title = task.title.trim();
    if title.is_empty() || title.chars().count() > TASK_TITLE_MAX_CHARS {
        return Err(StoreError::Internal(format!(
            "tiêu đề task phải 1..={TASK_TITLE_MAX_CHARS} ký tự"
        )));
    }
    if task.notes.chars().count() > NOTES_MAX_CHARS {
        return Err(StoreError::Internal(format!(
            "notes tối đa {NOTES_MAX_CHARS} ký tự"
        )));
    }
    validate_priority(&task.priority)?;
    let project_id_i64 = id_to_sql(task.project_id)?;
    // FK của SQLite chặn project không tồn tại, nhưng đổi lỗi thô thành câu
    // đọc được để model tự sửa (project_id sai là lỗi tham số, không phải lỗi hệ thống).
    let project_exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM work_projects WHERE id = ?1",
            params![project_id_i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(internal)?;
    if project_exists.is_none() {
        return Err(StoreError::Internal(format!(
            "project id={} không tồn tại — hãy `project_list` trước khi thêm task",
            task.project_id
        )));
    }
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO work_tasks
           (project_id, title, notes, status, priority, assignee, created_by, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'open', ?4, ?5, ?6, ?7, ?7)",
        params![
            project_id_i64,
            title,
            task.notes.trim(),
            task.priority,
            task.assignee.trim(),
            task.created_by,
            now
        ],
    )
    .map_err(|err| StoreError::Internal(format!("tạo task thất bại: {err}")))?;
    let id = conn.last_insert_rowid();
    Ok(WorkTask {
        id: id
            .try_into()
            .map_err(|_| StoreError::Internal("work_tasks.last_insert_rowid âm".into()))?,
        project_id: task.project_id,
        title: title.to_string(),
        notes: task.notes.trim().to_string(),
        status: "open".into(),
        priority: task.priority,
        assignee: task.assignee.trim().to_string(),
        created_by: task.created_by,
        created_at: now.clone(),
        updated_at: now,
        done_at: None,
    })
}

pub(super) fn update_work_task(
    conn: &Connection,
    id: u64,
    update: &WorkTaskUpdate,
) -> Result<Option<WorkTask>, StoreError> {
    let id_i64 = id_to_sql(id)?;
    let mut sets: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(title) = &update.title {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > TASK_TITLE_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "tiêu đề task phải 1..={TASK_TITLE_MAX_CHARS} ký tự"
            )));
        }
        args.push(Box::new(title.to_string()));
        sets.push(format!("title = ?{}", args.len()));
    }
    if let Some(notes) = &update.notes {
        if notes.chars().count() > NOTES_MAX_CHARS {
            return Err(StoreError::Internal(format!(
                "notes tối đa {NOTES_MAX_CHARS} ký tự"
            )));
        }
        args.push(Box::new(notes.trim().to_string()));
        sets.push(format!("notes = ?{}", args.len()));
    }
    if let Some(status) = &update.status {
        validate_status(status)?;
        args.push(Box::new(status.clone()));
        sets.push(format!("status = ?{}", args.len()));
        if status == "done" {
            args.push(Box::new(now_rfc3339()));
            sets.push(format!("done_at = ?{}", args.len()));
        } else {
            sets.push("done_at = NULL".into());
        }
    }
    if let Some(priority) = &update.priority {
        validate_priority(priority)?;
        args.push(Box::new(priority.clone()));
        sets.push(format!("priority = ?{}", args.len()));
    }
    if let Some(assignee) = &update.assignee {
        args.push(Box::new(assignee.trim().to_string()));
        sets.push(format!("assignee = ?{}", args.len()));
    }
    if sets.is_empty() {
        // Không có gì đổi: vẫn trả bản ghi hiện tại để caller khỏi truy vấn lại.
        return conn
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM work_tasks WHERE id = ?1"),
                params![id_i64],
                task_from_row,
            )
            .optional()
            .map_err(internal);
    }
    args.push(Box::new(now_rfc3339()));
    sets.push(format!("updated_at = ?{}", args.len()));
    let sql = format!(
        "UPDATE work_tasks SET {} WHERE id = ?{}",
        sets.join(", "),
        args.len() + 1
    );
    args.push(Box::new(id_i64));
    let changed = conn
        .execute(&sql, params_from_iter(args))
        .map_err(internal)?;
    if changed == 0 {
        return Ok(None);
    }
    conn.query_row(
        &format!("SELECT {TASK_COLUMNS} FROM work_tasks WHERE id = ?1"),
        params![id_i64],
        task_from_row,
    )
    .optional()
    .map_err(internal)
}

pub(super) fn delete_work_task(conn: &Connection, id: u64) -> Result<bool, StoreError> {
    let id_i64 = id_to_sql(id)?;
    let changed = conn
        .execute("DELETE FROM work_tasks WHERE id = ?1", params![id_i64])
        .map_err(internal)?;
    Ok(changed > 0)
}

pub(super) fn open_work_tasks(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<WorkTask>, StoreError> {
    let limit = limit.clamp(1, 50);
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM work_tasks
         WHERE status IN ('open', 'in_progress', 'blocked')
         ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END, id DESC
         LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql).map_err(internal)?;
    let rows = stmt.query_map([], task_from_row).map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}
