//! Truy vấn Bean World — nhóm **project**: liệt kê, tạo (tên duy nhất), archive.
//! Luật validate + helper nằm ở `q_bw.rs` (một nguồn chân lý dùng chung).

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use super::q_bw::{NOTES_MAX_CHARS, PROJECT_NAME_MAX_CHARS, id_to_sql};
use crate::store::shared::now_rfc3339;
use crate::store::types::{NewWorkProject, StoreError, WorkProject};

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkProject> {
    Ok(WorkProject {
        id: row.get::<_, i64>(0)?.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                "project id âm".into(),
            )
        })?,
        name: row.get(1)?,
        description: row.get(2)?,
        status: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

pub(super) fn list_projects(conn: &Connection) -> Result<Vec<WorkProject>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT id, name, description, status, created_at, updated_at FROM work_projects ORDER BY id DESC")
        .map_err(internal)?;
    let rows = stmt.query_map([], project_from_row).map_err(internal)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(internal)
}

pub(super) fn create_project(
    conn: &Connection,
    project: NewWorkProject,
) -> Result<WorkProject, StoreError> {
    let name = project.name.trim();
    if name.is_empty() || name.chars().count() > PROJECT_NAME_MAX_CHARS {
        return Err(StoreError::Internal(format!(
            "tên project phải 1..={PROJECT_NAME_MAX_CHARS} ký tự"
        )));
    }
    if project.description.chars().count() > NOTES_MAX_CHARS {
        return Err(StoreError::Internal(format!(
            "mô tả project tối đa {NOTES_MAX_CHARS} ký tự"
        )));
    }
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO work_projects (name, description, status, created_at, updated_at)
         VALUES (?1, ?2, 'active', ?3, ?3)",
        params![name, project.description.trim(), now],
    )
    .map_err(|err| match err {
        rusqlite::Error::SqliteFailure(e, _)
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::Internal(format!("project `{name}` đã tồn tại"))
        }
        other => StoreError::Internal(format!("tạo project thất bại: {other}")),
    })?;
    let id = conn.last_insert_rowid();
    Ok(WorkProject {
        id: id
            .try_into()
            .map_err(|_| StoreError::Internal("work_projects.last_insert_rowid âm".into()))?,
        name: name.to_string(),
        description: project.description.trim().to_string(),
        status: "active".into(),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub(super) fn set_project_status(
    conn: &Connection,
    id: u64,
    status: &str,
) -> Result<Option<WorkProject>, StoreError> {
    if status != "active" && status != "archived" {
        return Err(StoreError::Internal(
            "trạng thái project chỉ nhận: active, archived".into(),
        ));
    }
    let id_i64 = id_to_sql(id)?;
    let now = now_rfc3339();
    let changed = conn
        .execute(
            "UPDATE work_projects SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now, id_i64],
        )
        .map_err(internal)?;
    if changed == 0 {
        return Ok(None);
    }
    conn.query_row(
        "SELECT id, name, description, status, created_at, updated_at FROM work_projects WHERE id = ?1",
        params![id_i64],
        project_from_row,
    )
    .optional()
    .map_err(internal)
}
