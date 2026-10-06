//! Truy van token client MCP (M25) — `mcp_clients`, luu **hash** token, khong luu token.

use rusqlite::{Connection, OptionalExtension, params};

use super::migration::internal;
use crate::store::types::{McpClientInfo, StoreError};

// ---------------------------------------------------------------------------
// Token client MCP (M25)
// ---------------------------------------------------------------------------

/// Cấp token client MCP. Cấp lại cho cùng tên ⇒ xoá bản ghi cũ, nên không bao giờ
/// có hai token cùng lúc cho một client (đồng bộ với `MemoryStore`).
pub(super) fn create_mcp_client(
    conn: &Connection,
    token_hash: &str,
    name: &str,
    role: &str,
    created_at: &str,
    expires_at: &str,
) -> Result<(), StoreError> {
    let tx = conn.unchecked_transaction().map_err(internal)?;
    tx.execute("DELETE FROM mcp_clients WHERE name = ?1", params![name])
        .map_err(internal)?;
    tx.execute(
        "INSERT INTO mcp_clients(token_hash, name, role, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![token_hash, name, role, created_at, expires_at],
    )
    .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(())
}

/// `expires_at` rỗng ⇒ không hết hạn (giống `MemoryStore`).
pub(super) fn get_mcp_client(
    conn: &Connection,
    token_hash: &str,
    now: &str,
) -> Result<Option<McpClientInfo>, StoreError> {
    conn.query_row(
        "SELECT name, role, created_at, expires_at FROM mcp_clients \
         WHERE token_hash = ?1 AND (expires_at = '' OR expires_at > ?2)",
        params![token_hash, now],
        |row| {
            Ok(McpClientInfo {
                name: row.get(0)?,
                role: row.get(1)?,
                created_at: row.get(2)?,
                expires_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(internal)
}

pub(super) fn list_mcp_clients(conn: &Connection) -> Result<Vec<McpClientInfo>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT name, role, created_at, expires_at FROM mcp_clients ORDER BY created_at, name",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok(McpClientInfo {
                name: row.get(0)?,
                role: row.get(1)?,
                created_at: row.get(2)?,
                expires_at: row.get(3)?,
            })
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(internal)?);
    }
    Ok(out)
}

pub(super) fn delete_mcp_client(conn: &Connection, name: &str) -> Result<bool, StoreError> {
    Ok(conn
        .execute("DELETE FROM mcp_clients WHERE name = ?1", params![name])
        .map_err(internal)?
        > 0)
}
