#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use beanagent_memory::{MemoryStore, NewScheduledTask, SqliteStore, Store};

fn input() -> NewScheduledTask {
    NewScheduledTask {
        cron: "0 9 * * *".into(),
        prompt: "daily summary".into(),
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
