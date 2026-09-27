#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use beanagent_memory::{MemoryStore, NewScheduledTask, SqliteStore, Store};

fn input() -> NewScheduledTask {
    NewScheduledTask {
        cron: "0 9 * * *".into(),
        prompt: "daily summary".into(),
        session_id: None,
        channel: "web".into(),
        chat_id: "web:admin".into(),
        allowed_tools: vec!["web_fetch".into()],
        next_run: "2026-09-26T09:00:00Z".into(),
        enabled: true,
    }
}

#[tokio::test]
async fn memory_store_task_crud() {
    let store = MemoryStore::new();
    let created = store.create_task(input()).await.unwrap();
    assert_eq!(created.id, 1);
    assert_eq!(store.list_tasks().await.unwrap().len(), 1);
    let toggled = store
        .set_task_enabled(created.id, false)
        .await
        .unwrap()
        .unwrap();
    assert!(!toggled.enabled);
    assert!(store.delete_task(created.id).await.unwrap());
    assert!(store.list_tasks().await.unwrap().is_empty());
}

#[tokio::test]
async fn sqlite_store_task_crud_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tasks.db");
    let store = SqliteStore::open(&path).unwrap();
    let created = store.create_task(input()).await.unwrap();
    drop(store);

    let reopened = SqliteStore::open(&path).unwrap();
    let listed = reopened.list_tasks().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].prompt, "daily summary");
    assert_eq!(listed[0].allowed_tools, vec!["web_fetch".to_string()]);
    assert!(
        reopened
            .set_task_enabled(created.id, false)
            .await
            .unwrap()
            .is_some()
    );
    assert!(reopened.delete_task(created.id).await.unwrap());
}

// ---------------------------------------------------------------------------
// M25 — token client MCP: cùng ngữ nghĩa trên `MemoryStore` và `SqliteStore` (K7), và
// sống sót qua lần mở lại DB (migration v6).
// ---------------------------------------------------------------------------

/// Bộ test hành vi chạy cho **cả hai** cài đặt `Store` — K7 ghi rõ hai bản này dễ trôi
/// lệch nhau, nên mọi khác biệt phải được bắt ở đây chứ không phải lúc chạy thật.
async fn mcp_client_behaviour(store: &dyn Store) {
    let now = "2026-09-27T00:00:00+00:00";
    let hash_a = "a".repeat(64);
    let hash_b = "b".repeat(64);
    assert!(store.list_mcp_clients().await.unwrap().is_empty());

    store
        .create_mcp_client(&hash_a, "cline", "monitor", now, "")
        .await
        .unwrap();
    store
        .create_mcp_client(
            &hash_b,
            "ops",
            "finance-readonly",
            now,
            "2030-01-01T00:00:00+00:00",
        )
        .await
        .unwrap();
    let listed = store.list_mcp_clients().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].name, "cline");
    assert_eq!(listed[0].role, "monitor");
    // `expires_at` rỗng ⇒ không hết hạn.
    assert!(listed[0].expires_at.is_empty());

    // Token đúng ⇒ tra ra client.
    let found = store
        .get_mcp_client(&hash_a, "2026-09-28T00:00:00+00:00")
        .await
        .unwrap()
        .expect("token đúng phải tra được");
    assert_eq!(found.name, "cline");
    // Token sai ⇒ `None` (không lỗi, không panic).
    assert!(
        store
            .get_mcp_client(&"c".repeat(64), "2026-09-28T00:00:00+00:00")
            .await
            .unwrap()
            .is_none()
    );
    // Token hết hạn ⇒ `None`.
    assert!(
        store
            .get_mcp_client(&hash_b, "2031-01-01T00:00:00+00:00")
            .await
            .unwrap()
            .is_none()
    );

    // Cấp lại cho cùng tên ⇒ token cũ mất hiệu lực, không tồn tại hai token cho
    // một client (nếu không, `revoke` sẽ chỉ xoá một trong hai).
    let hash_c = "c".repeat(64);
    store
        .create_mcp_client(&hash_c, "cline", "monitor", now, "")
        .await
        .unwrap();
    assert!(
        store
            .get_mcp_client(&hash_a, "2026-09-28T00:00:00+00:00")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .get_mcp_client(&hash_c, "2026-09-28T00:00:00+00:00")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(store.list_mcp_clients().await.unwrap().len(), 2);

    // Thu hồi.
    assert!(store.delete_mcp_client("cline").await.unwrap());
    assert!(!store.delete_mcp_client("cline").await.unwrap());
    assert!(!store.delete_mcp_client("khong-co").await.unwrap());
    assert_eq!(store.list_mcp_clients().await.unwrap().len(), 1);
}

#[tokio::test]
async fn memory_store_mcp_clients() {
    mcp_client_behaviour(&MemoryStore::new()).await;
}

#[tokio::test]
async fn sqlite_store_mcp_clients_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.db");
    let store = SqliteStore::open(&path).unwrap();
    store
        .create_mcp_client(
            &"d".repeat(64),
            "cline",
            "monitor",
            "2026-09-27T00:00:00+00:00",
            "",
        )
        .await
        .unwrap();
    drop(store);

    // Mở lại: migration v6 phải chạy được trên DB cũ và dữ liệu còn nguyên.
    let store = SqliteStore::open(&path).unwrap();
    let found = store
        .get_mcp_client(&"d".repeat(64), "2026-09-28T00:00:00+00:00")
        .await
        .unwrap()
        .expect("token phải sống sót qua lần mở lại");
    assert_eq!(found.name, "cline");
    assert_eq!(store.list_mcp_clients().await.unwrap().len(), 1);
}
