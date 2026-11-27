//! Bean World (M-BeanWorld): bảng công việc `Project → Task → Note` dùng chung
//! giữa người thật (REST/UI) và agent (tool `work_*`).
//!
//! Điểm dễ lệch giữa hai bản cài đặt: **cascade** (xoá task phải xoá ghi chú),
//! thứ tự sắp xếp (priority rồi mới nhất) và `done_at` (đặt khi vào `done`,
//! xoá khi rời `done`). Mọi khẳng định dưới đây chạy trên **cả hai** store.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::store::types::{
    NewWorkNote, NewWorkProject, NewWorkTask, StoreError, WorkTaskFilter, WorkTaskUpdate,
};
use crate::traits::WorkStore;

/// Kịch bản trọn vòng đời project → task → note → xoá, chạy trên một store.
async fn roundtrip<W: WorkStore>(store: &W) {
    // --- Project: tạo + trùng tên ---
    let project = store
        .create_project(NewWorkProject {
            name: "bean world".into(),
            description: "bảng việc chung".into(),
        })
        .await
        .unwrap();
    assert_eq!(project.status, "active");

    let duplicate = store
        .create_project(NewWorkProject {
            name: "bean world".into(),
            description: String::new(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(duplicate, StoreError::Internal(ref message) if message.contains("đã tồn tại")),
        "trùng tên project phải báo lỗi rõ ràng: {duplicate:?}"
    );

    // --- Task: project phải tồn tại ---
    let missing_project = store
        .create_work_task(NewWorkTask {
            project_id: project.id + 9_999,
            title: "việc ma".into(),
            notes: String::new(),
            assignee: String::new(),
            priority: "normal".into(),
            created_by: "user:test".into(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(missing_project, StoreError::Internal(ref message) if message.contains("không tồn tại")),
        "task trên project không tồn tại phải báo lỗi: {missing_project:?}"
    );

    // --- Ba task để kiểm tra lọc + thứ tự ---
    let high = store
        .create_work_task(NewWorkTask {
            project_id: project.id,
            title: "việc ưu tiên cao".into(),
            notes: String::new(),
            assignee: "developer".into(),
            priority: "high".into(),
            created_by: "user:test".into(),
        })
        .await
        .unwrap();
    let normal = store
        .create_work_task(NewWorkTask {
            project_id: project.id,
            title: "việc thường".into(),
            notes: "bối cảnh".into(),
            assignee: String::new(),
            priority: "normal".into(),
            created_by: "user:test".into(),
        })
        .await
        .unwrap();
    let last = store
        .create_work_task(NewWorkTask {
            project_id: project.id,
            title: "việc muộn hơn".into(),
            notes: String::new(),
            assignee: "qa".into(),
            priority: "normal".into(),
            created_by: "agent".into(),
        })
        .await
        .unwrap();

    let by_project = store
        .list_work_tasks(WorkTaskFilter {
            project_id: Some(project.id),
            assignee: None,
            status: None,
        })
        .await
        .unwrap();
    assert_eq!(by_project.len(), 3, "lọc theo project");
    assert_eq!(by_project[0].id, last.id, "mới nhất trước");

    let by_assignee = store
        .list_work_tasks(WorkTaskFilter {
            project_id: None,
            assignee: Some("developer".into()),
            status: None,
        })
        .await
        .unwrap();
    assert_eq!(by_assignee.len(), 1);
    assert_eq!(by_assignee[0].id, high.id);

    let open_only = store
        .list_work_tasks(WorkTaskFilter {
            project_id: None,
            assignee: None,
            status: Some("open".into()),
        })
        .await
        .unwrap();
    assert_eq!(open_only.len(), 3);

    // --- done_at: đặt khi vào done, xoá khi rời done ---
    let done = store
        .update_work_task(
            normal.id,
            WorkTaskUpdate {
                status: Some("done".into()),
                ..WorkTaskUpdate::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, "done");
    assert!(done.done_at.is_some(), "vào done phải đóng dấu thời điểm");

    let reopened = store
        .update_work_task(
            normal.id,
            WorkTaskUpdate {
                status: Some("open".into()),
                ..WorkTaskUpdate::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(reopened.done_at.is_none(), "rời done phải xoá done_at");

    // Cập nhật rỗng = đọc lại bản ghi hiện tại.
    let unchanged = store
        .update_work_task(normal.id, WorkTaskUpdate::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.status, "open");

    // Trạng thái ngoài tập bị từ chối (store là rào chặn cuối).
    let bad_status = store
        .update_work_task(
            normal.id,
            WorkTaskUpdate {
                status: Some("hoàn thành".into()),
                ..WorkTaskUpdate::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(bad_status, StoreError::Internal(ref message) if message.contains("không hợp lệ")),
        "trạng thái lạ phải bị từ chối: {bad_status:?}"
    );

    // --- open_work_tasks: ưu tiên high lên đầu, done bị loại ---
    let open_list = store.open_work_tasks(50).await.unwrap();
    assert!(
        open_list.iter().all(|task| task.status != "done"),
        "done không còn mở"
    );
    assert_eq!(open_list[0].id, high.id, "priority cao đứng trước");
    assert!(
        store.open_work_tasks(2).await.unwrap().len() <= 2,
        "limit bị tôn trọng"
    );

    // --- Notes: append-only, cũ nhất trước, task phải tồn tại ---
    let first_note = store
        .add_work_note(NewWorkNote {
            task_id: high.id,
            author: "user:admin".into(),
            text: "giao việc xong".into(),
        })
        .await
        .unwrap();
    let second_note = store
        .add_work_note(NewWorkNote {
            task_id: high.id,
            author: "agent".into(),
            text: "đã nhận".into(),
        })
        .await
        .unwrap();
    let notes = store.list_work_notes(high.id).await.unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].id, first_note.id, "cũ nhất trước");
    assert_eq!(notes[1].id, second_note.id);

    let note_on_missing = store
        .add_work_note(NewWorkNote {
            task_id: last.id + 9_999,
            author: "user:admin".into(),
            text: "xin chào".into(),
        })
        .await
        .unwrap_err();
    assert!(
        matches!(note_on_missing, StoreError::Internal(ref message) if message.contains("không tồn tại")),
        "note trên task không tồn tại phải báo lỗi: {note_on_missing:?}"
    );

    // --- Xoá task: ghi chú cascade theo (khớp FK CASCADE của SQLite) ---
    assert!(store.delete_work_task(last.id).await.unwrap());
    assert!(
        !store.delete_work_task(last.id).await.unwrap(),
        "xoá lần hai là no-op"
    );
    assert!(
        store.list_work_notes(last.id).await.unwrap().is_empty(),
        "xoá task phải xoá cả ghi chú của nó"
    );

    // --- Archive project ---
    let archived = store
        .set_project_status(project.id, "archived")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archived.status, "archived");
    assert!(
        store
            .set_project_status(project.id + 9_999, "archived")
            .await
            .unwrap()
            .is_none(),
        "project không tồn tại trả None"
    );
}

#[tokio::test]
async fn sqlite_store_work_roundtrip() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = super::open(&dir);
    roundtrip(&store).await;
}

#[tokio::test]
async fn memory_store_work_roundtrip() {
    let store = crate::store::MemoryStore::new();
    roundtrip(&store).await;
}
