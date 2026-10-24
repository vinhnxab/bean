//! Truy vấn bật/tắt tool runtime (`tool_overrides`) — hàng tồn tại = override 2 chiều.

use rusqlite::{Connection, params};

use super::migration::internal;
use crate::store::shared::now_rfc3339;
use crate::store::types::StoreError;
use crate::traits::traits::ToolOverride;

// ---------------------------------------------------------------------------
// Tool override (runtime toggle từ UI — thắng seed `[tools].enabled`)
// ---------------------------------------------------------------------------

/// Ghi đè trạng thái tool: `enabled = true` bật, `false` tắt — luôn UPSERT hàng.
///
/// Khác v7 (hàng tồn tại = bị tắt): override giờ **2 chiều** nên bật không còn xoá
/// hàng — nếu xoá, tool quay về trạng thái seed và việc "bật từ UI" trên tool bị
/// seed tắt sẽ mất tác dụng.
pub(super) fn set_tool_enabled(
    conn: &Connection,
    name: &str,
    enabled: bool,
    updated_by: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO tool_overrides(name, enabled, updated_at, updated_by) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(name) DO UPDATE SET enabled = excluded.enabled, \
         updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        params![name, i64::from(enabled), now_rfc3339(), updated_by],
    )
    .map_err(internal)?;
    Ok(())
}

/// Mọi override đã lưu (sort theo tên), 2 chiều bật/tắt.
pub(super) fn tool_overrides(conn: &Connection) -> Result<Vec<ToolOverride>, StoreError> {
    let mut statement = conn
        .prepare("SELECT name, enabled FROM tool_overrides ORDER BY name")
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok(ToolOverride {
                name: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
            })
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(internal)?);
    }
    Ok(out)
}
