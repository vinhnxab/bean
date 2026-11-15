//! Vòng đời phiên: tái dùng/archive (`/new`), ownership theo `user_id`, tiêu đề
//! từ tin đầu tiên, và hợp đồng chung hai bản cho xoá/archive/liệt kê (K7).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bean_types::Message;
use tempfile::TempDir;

use super::{db_path, open};
use crate::store::{MemoryStore, Store};
// Method gọi trên struct cụ thể cần sub-trait tương ứng trong scope.
use crate::traits::{MessageStore, SessionStore};

/// `/new`: lưu trữ phiên cũ (không xoá) rồi mở phiên mới cho cùng `chat_id`.
#[tokio::test]
async fn ensure_session_reuses_active_then_creates_new_after_archive() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let first = store.ensure_session("cli", "local", "").await.unwrap();
    assert_eq!(
        first,
        store.ensure_session("cli", "local", "").await.unwrap()
    );
    let other = store.ensure_session("cli", "khác", "").await.unwrap();
    assert_ne!(first, other);

    store.archive_session(first).await.unwrap();
    let next = store.ensure_session("cli", "local", "").await.unwrap();
    assert_ne!(next, first);
    store.append(next, Message::user("xin chào")).await.unwrap();
    assert_eq!(store.count(next).await.unwrap(), 1);
    assert_eq!(store.count(first).await.unwrap(), 0);
    assert_eq!(store.count(other).await.unwrap(), 0);
}

/// `user_id` gắn ownership session; user khác không nhận lại session cũ.
#[tokio::test]
async fn ensure_session_for_user_enforces_ownership() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let first = store
        .ensure_session_for_user("web", "chat-a", "web:admin", "")
        .await
        .unwrap();
    let info = store.session_info(first).await.unwrap().unwrap();
    assert_eq!(info.user_id, "web:admin");
    assert!(!info.archived);
    assert_eq!(
        first,
        store
            .ensure_session_for_user("web", "chat-a", "web:admin", "")
            .await
            .unwrap()
    );
    let other = store
        .ensure_session_for_user("web", "chat-a", "web:other", "")
        .await
        .unwrap();
    assert_ne!(other, first);
}

/// Tiêu đề phiên lấy từ **dòng đầu** của tin đầu tiên do người dùng gửi (mục 8.1).
#[tokio::test]
async fn session_title_comes_from_first_user_message() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    store
        .append(
            session,
            Message::user("Lập kế hoạch cho tuần này\nchi tiết ở dưới"),
        )
        .await
        .unwrap();
    store
        .append(session, Message::user("tin thứ hai"))
        .await
        .unwrap();
    let conn = rusqlite::Connection::open(db_path(&dir)).unwrap();
    let title: String = conn
        .query_row(
            "SELECT title FROM sessions WHERE id = ?1",
            rusqlite::params![session.get()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Lập kế hoạch cho tuần này");
}

/// K7: `update_session`/`delete_session` chỉ thành công với đúng owner; xoá phiên
/// phải cascade message (SQLite qua `ON DELETE CASCADE` + `foreign_keys=ON`,
/// Memory qua struct giữ message bên trong session). `find_active_session` phải
/// khớp `ensure_session` và `None` sau archive.
#[tokio::test]
async fn session_ownership_lifecycle_in_both_stores() {
    async fn flow(store: &impl Store) {
        let session = store
            .ensure_session_for_user("web", "chat-1", "web:admin", "Tiêu đề gốc")
            .await
            .unwrap();
        store
            .append(session, Message::user("tin nhắn sẽ bị xoá cùng phiên"))
            .await
            .unwrap();

        // `session_info` trả ownership cho Router chống IDOR.
        let info = store.session_info(session).await.unwrap().unwrap();
        assert_eq!(info.user_id, "web:admin");
        assert_eq!(info.channel, "web");
        assert_eq!(info.chat_id, "chat-1");

        // Sai owner: không sửa được, không xoá được — trả `false`, không lỗi.
        assert!(
            !store
                .update_session(session, "web:other", Some("giựt"), None)
                .await
                .unwrap()
        );
        assert!(!store.delete_session(session, "web:other").await.unwrap());

        // Đúng owner: đổi tên + archive thành công, thấy qua `session_summary`.
        assert!(
            store
                .update_session(session, "web:admin", Some("Tiêu đề mới"), Some(true))
                .await
                .unwrap()
        );
        let summary = store.session_summary(session).await.unwrap().unwrap();
        assert_eq!(summary.title, "Tiêu đề mới");
        assert!(summary.archived);

        // Archive rồi `find_active_session` phải `None` (không còn phiên active).
        assert!(
            store
                .find_active_session("web", "chat-1")
                .await
                .unwrap()
                .is_none()
        );

        // Xoá đúng owner: message bị cascade theo — `count` lỗi khi phiên mất.
        assert!(store.delete_session(session, "web:admin").await.unwrap());
        assert!(store.session_info(session).await.unwrap().is_none());
        assert!(store.session_summary(session).await.unwrap().is_none());
        assert_eq!(store.count(session).await.unwrap(), 0);
        assert!(
            store
                .list_message_records(session, None, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let dir = TempDir::new().unwrap();
    flow(&open(&dir)).await;
    flow(&MemoryStore::new()).await;
}

/// K7: `list_sessions` phải giống nhau trên cả hai bản — lọc theo user, tìm
/// tiêu đề **case-insensitive cả Unicode** ("TỔNG" thấy "Tổng"), lọc archived
/// và tôn trọng limit. So sánh dạng tập: MemoryStore không có updated_at nên
/// thứ tự trả về là chi tiết riêng của từng bản, không phải hợp đồng chung.
#[tokio::test]
async fn list_sessions_filters_and_limits_in_both_stores() {
    async fn flow(store: &impl Store) {
        let total = store
            .ensure_session_for_user("web", "c1", "web:admin", "Tổng kết tuần")
            .await
            .unwrap();
        let other_archived = store
            .ensure_session_for_user("web", "c2", "web:admin", "Báo cáo hôm nay")
            .await
            .unwrap();
        let tech = store
            .ensure_session_for_user("web", "c3", "web:admin", "tài liệu kỹ thuật")
            .await
            .unwrap();
        store
            .ensure_session_for_user("web", "c4", "web:other", "phiên người khác")
            .await
            .unwrap();
        store.archive_session(other_archived).await.unwrap();

        let ids = |list: Vec<crate::SessionSummary>| -> Vec<bean_types::SessionId> {
            list.into_iter().map(|item| item.id).collect()
        };
        // So dạng tập: MemoryStore không có updated_at nên thứ tự trả về là
        // chi tiết riêng của từng bản, không phải hợp đồng chung.
        let sorted = |mut v: Vec<bean_types::SessionId>| -> Vec<bean_types::SessionId> {
            v.sort();
            v
        };

        // Mặc định: chỉ phiên của user, chưa archived.
        let active = ids(store
            .list_sessions("web:admin", None, Some(false), 10)
            .await
            .unwrap());
        assert_eq!(sorted(active), sorted(vec![total, tech]));

        // Tìm tiêu đề: hoa/thường khác nhau cả ASCII lẫn dấu tiếng Việt.
        let found = store
            .list_sessions("web:admin", Some("tỔNG"), None, 10)
            .await
            .unwrap();
        assert_eq!(
            ids(found),
            vec![total],
            "K7: tìm tiêu đề phải Unicode case-insensitive"
        );
        let found = store
            .list_sessions("web:admin", Some("BÁO CÁO"), Some(true), 10)
            .await
            .unwrap();
        assert_eq!(ids(found), vec![other_archived]);

        // Không thấy phiên của user khác; limit cắt bớt khi có nhiều hơn.
        assert!(
            store
                .list_sessions("web:admin", Some("người khác"), None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .list_sessions("web:admin", None, Some(true), 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    let dir = TempDir::new().unwrap();
    flow(&open(&dir)).await;
    flow(&MemoryStore::new()).await;
}
