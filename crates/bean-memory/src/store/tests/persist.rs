//! Bền vững (agents.md mục 20): dữ liệu sống qua khởi động lại, DB dở dang mở
//! được, migration v2/v7 chịu được trạng thái cũ, outbox retry bền vững.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bean_types::{Message, Outbound, OutboundKind, Role, SessionId};
use tempfile::TempDir;

use super::{db_path, open};
use crate::store::MemorySource;
use crate::store::sqlite::migration::configure;
use crate::store::sqlite::schema::SCHEMA_SQL;
// Method gọi trên struct cụ thể cần sub-trait tương ứng trong scope.
use crate::traits::{
    MemoryStore as _, MessageStore, OutboxStore, SessionStore, ToolOverride, ToolOverrideStore,
};

/// Bền vững qua khởi động lại (agents.md mục 20).
#[tokio::test]
async fn sqlite_persists_messages_across_restart() {
    let dir = TempDir::new().unwrap();
    let session;
    {
        let store = open(&dir);
        session = store.ensure_session("cli", "local", "").await.unwrap();
        store
            .append(session, Message::user("nhớ giúp tôi việc này"))
            .await
            .unwrap();
        store
            .append(session, Message::tool("c1", "kết quả"))
            .await
            .unwrap();
        store
            .memory_save("người dùng tên Vinh", "hồ sơ")
            .await
            .unwrap();
        store.save_summary(session, "tóm tắt cũ").await.unwrap();
    } // drop ⇒ worker dừng, connection đóng (WAL flush)

    let reopened = open(&dir);
    let session_again = reopened.ensure_session("cli", "local", "").await.unwrap();
    assert_eq!(
        session_again, session,
        "phiên phải được tái sử dụng sau restart"
    );
    let history = reopened.history(session, None, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].text.as_deref(), Some("nhớ giúp tôi việc này"));
    assert_eq!(history[1].role, Role::Tool);
    assert_eq!(history[1].tool_call_id.as_deref(), Some("c1"));
    assert_eq!(
        reopened.summary(session).await.unwrap().as_deref(),
        Some("tóm tắt cũ")
    );
    let hits = reopened.memory_search("Vinh", None).await.unwrap();
    assert!(hits.iter().any(|h| h.source == MemorySource::Memories));
}

/// Outbox bền vững: ghi lỗi, tăng attempts, đổi lịch rồi xoá khi gửi thành công.
#[tokio::test]
async fn sqlite_outbox_retry_lifecycle_survives_restart() {
    let dir = TempDir::new().unwrap();
    let payload = Outbound {
        session_id: SessionId::new(7),
        message_id: 42,
        text: "tin chủ động 🧸".into(),
        kind: OutboundKind::Notification,
        action: None,
    };
    let id = {
        let store = open(&dir);
        let id = store
            .enqueue_outbound("telegram", "123", &payload, "2999-01-01T00:00:00.000000Z")
            .await
            .unwrap();
        let due = store
            .due_outbox("3000-01-01T00:00:00.000000Z", 10)
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].payload, payload);
        store
            .retry_outbox(id, "2999-01-01T00:00:00.000000Z", "mạng lỗi: timeout")
            .await
            .unwrap();
        id
    };

    let reopened = open(&dir);
    let due = reopened
        .due_outbox("3000-01-01T00:00:00.000000Z", 10)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].id, id);
    assert_eq!(due[0].attempts, 1);
    assert_eq!(due[0].last_error, "mạng lỗi: timeout");
    reopened.complete_outbox(id).await.unwrap();
    assert!(
        reopened
            .due_outbox("3000-01-01T00:00:00.000000Z", 10)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Migration v2 chịu được DB bị dừng giữa chừng sau khi đã thêm một cột.
#[tokio::test]
async fn migration_v2_resumes_after_partial_schema_change() {
    let dir = TempDir::new().unwrap();
    {
        let conn = rusqlite::Connection::open(db_path(&dir)).unwrap();
        configure(&conn).unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        conn.execute_batch("ALTER TABLE sessions ADD COLUMN user_id TEXT NOT NULL DEFAULT ''")
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let store = open(&dir);
    let session = store
        .ensure_session_for_user("cli", "local", "cli:local", "")
        .await
        .unwrap();
    assert_eq!(
        store.session_info(session).await.unwrap().unwrap().user_id,
        "cli:local"
    );
    let version: i64 = rusqlite::Connection::open(db_path(&dir))
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    // 10 = đã chạy tới migration mới nhất (v8 `tool_overrides` runtime toggle,
    // v9 `push_subscriptions` cho Web Push, v10 `work_*` cho Bean World).
    assert_eq!(version, 10);
}

/// Migration v7 → v8: DB cũ (hàng tồn tại = tool bị tắt, không cột `enabled`)
/// phải mở được và hàng cũ trở thành `enabled = false` — đúng nghĩa cũ, không mất.
#[tokio::test]
async fn migration_v7_rows_become_disabled_overrides_in_v8() {
    let dir = TempDir::new().unwrap();
    {
        let conn = rusqlite::Connection::open(db_path(&dir)).unwrap();
        configure(&conn).unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        // Giả lập DB v7: bảng không cột `enabled`, một hàng = tool bị tắt.
        conn.execute_batch(
            "ALTER TABLE tool_overrides DROP COLUMN enabled;
             INSERT INTO tool_overrides(name, updated_at, updated_by)
               VALUES ('run_shell', '2026-10-22T00:00:00Z', 'web:admin');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();
    }
    let store = open(&dir);
    assert_eq!(
        store.tool_overrides().await.unwrap(),
        vec![ToolOverride {
            name: "run_shell".to_string(),
            enabled: false,
        }],
        "hàng v7 (tồn tại = bị tắt) thành override enabled = false"
    );
    // Ghi mới sau migration dùng schema v8 bình thường.
    store
        .set_tool_enabled("run_shell", true, "web:admin")
        .await
        .unwrap();
    assert_eq!(
        store.tool_overrides().await.unwrap(),
        vec![ToolOverride {
            name: "run_shell".to_string(),
            enabled: true,
        }]
    );
}
