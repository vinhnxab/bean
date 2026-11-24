//! Truy vấn subscription Web Push (VAPID) — bảng `push_subscriptions`.
//!
//! Endpoint là khoá chính: một trình duyệt chỉ có một subscription cho một
//! origin + service worker, đăng ký lại là cập nhật key (UPSERT). Số lần gửi
//! lỗi là trạng thái của **server**, nên UPSERT không đụng tới nó.

use rusqlite::{Connection, params};

use super::migration::internal;
use crate::store::types::{NewPushSubscription, PushSubscription, StoreError};

/// Lưu subscription; đăng ký lại cùng endpoint cập nhật key/user-agent.
pub(super) fn upsert_push_subscription(
    conn: &Connection,
    sub: &NewPushSubscription,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO push_subscriptions(endpoint, p256dh, auth, user_agent, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(endpoint) DO UPDATE SET \
           p256dh = excluded.p256dh, auth = excluded.auth, user_agent = excluded.user_agent",
        params![
            sub.endpoint,
            sub.p256dh,
            sub.auth,
            sub.user_agent,
            sub.created_at
        ],
    )
    .map_err(internal)?;
    Ok(())
}

/// Mọi subscription, cũ nhất trước — dispatcher duyệt tuần tự.
pub(super) fn list_push_subscriptions(
    conn: &Connection,
) -> Result<Vec<PushSubscription>, StoreError> {
    let mut statement = conn
        .prepare(
            "SELECT endpoint, p256dh, auth, user_agent, created_at, failure_count \
             FROM push_subscriptions ORDER BY created_at, endpoint",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok(PushSubscription {
                endpoint: row.get(0)?,
                p256dh: row.get(1)?,
                auth: row.get(2)?,
                user_agent: row.get(3)?,
                created_at: row.get(4)?,
                failure_count: row.get::<_, i64>(5)?.clamp(0, u32::MAX as i64) as u32,
            })
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(internal)?);
    }
    Ok(out)
}

pub(super) fn delete_push_subscription(
    conn: &Connection,
    endpoint: &str,
) -> Result<bool, StoreError> {
    Ok(conn
        .execute(
            "DELETE FROM push_subscriptions WHERE endpoint = ?1",
            params![endpoint],
        )
        .map_err(internal)?
        > 0)
}

/// Gửi thành công reset số lỗi; thất bại cộng dồn (trần u32 để không tràn khi
/// subscription hỏng dai dẳng mà chưa bị xoá).
pub(super) fn record_push_outcome(
    conn: &Connection,
    endpoint: &str,
    ok: bool,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE push_subscriptions \
         SET failure_count = CASE WHEN ?2 THEN 0 ELSE MIN(failure_count + 1, 4294967295) END \
         WHERE endpoint = ?1",
        params![endpoint, ok],
    )
    .map_err(internal)?;
    Ok(())
}
