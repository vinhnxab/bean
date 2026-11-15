//! Hợp đồng chung hai bản cài đặt + ngữ nghĩa runtime: thao tác cơ bản
//! (M3 + M5), override tool 2 chiều, ngân sách token tách theo role (M21.7),
//! web session (K7) và outbox (K7).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bean_types::{Message, Outbound, OutboundKind, SessionId};
use tempfile::TempDir;

use super::open;
use crate::store::{MemorySource, MemoryStore, Store};
// Method gọi trên struct cụ thể cần sub-trait tương ứng trong scope.
use crate::traits::{
    MemoryStore as _, MessageStore, SessionStore, ToolOverride, ToolOverrideStore, UsageStore,
};

/// M21.7: ngân sách token **tách theo role** — role này dùng hết hạn mức thì role
/// khác vẫn còn ngân sách (tránh Developer ăn hết hạn mức của Monitor/Security-scan).
#[tokio::test]
async fn usage_by_role_is_independent_between_roles() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let usage = bean_types::Usage {
        input_tokens: 100,
        output_tokens: 50,
    };
    store
        .add_usage_by_role("2026-09-26", "developer", usage)
        .await
        .unwrap();
    store
        .add_usage_by_role("2026-09-26", "developer", usage)
        .await
        .unwrap();
    store
        .add_usage_by_role(
            "2026-09-26",
            "monitor",
            bean_types::Usage {
                input_tokens: 7,
                output_tokens: 3,
            },
        )
        .await
        .unwrap();

    let developer = store
        .usage_by_role("2026-09-26", "developer")
        .await
        .unwrap();
    assert_eq!(developer.input_tokens, 200);
    assert_eq!(developer.output_tokens, 100);

    let monitor = store.usage_by_role("2026-09-26", "monitor").await.unwrap();
    assert_eq!(monitor.total(), 10, "role khác không bị role này ăn hết");

    // Bảng tổng `usage` (phục vụ /api/status) vẫn tách biệt, không bị thay thế.
    assert_eq!(
        store.usage("2026-09-26").await.unwrap().total(),
        0,
        "ghi theo role không được đụng vào sổ tổng"
    );
    assert_eq!(
        store
            .usage_by_role("2026-09-26", "chua-ton-tai")
            .await
            .unwrap(),
        bean_types::Usage::default(),
        "role chưa từng dùng thì usage rỗng"
    );
}

/// Bản in-memory hành xử giống bản SQLite ở các thao tác cơ bản (M3 + M5).
#[tokio::test]
async fn memory_store_behaves_like_sqlite_for_basic_ops() {
    let store = MemoryStore::new();
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    assert_eq!(
        session,
        store.ensure_session("cli", "local", "").await.unwrap()
    );
    store.append(session, Message::user("một")).await.unwrap();
    store
        .append(session, Message::tool("c1", "hai"))
        .await
        .unwrap();
    assert_eq!(store.count(session).await.unwrap(), 2);
    let last = store.history(session, None, 1).await.unwrap();
    assert_eq!(last[0].text.as_deref(), Some("hai"));
    assert_eq!(
        store.list_messages(session, None, 0).await.unwrap()[1].seq,
        2
    );
    assert_eq!(
        store.memory_save("ghi nhớ về zeta", "tag").await.unwrap(),
        1
    );
    let hits = store.memory_search("zeta", None).await.unwrap();
    assert_eq!(hits[0].source, MemorySource::Memories);
    store.delete_before(session, 2).await.unwrap();
    assert_eq!(store.count(session).await.unwrap(), 1);
    store.archive_session(session).await.unwrap();
    assert_ne!(
        session,
        store.ensure_session("cli", "local", "").await.unwrap()
    );
    store.clear(session).await.unwrap();
    assert_eq!(store.count(session).await.unwrap(), 0);
    // `append` tự tạo phiên chưa biết (test M3 ghi thẳng `SessionId::new(99)`).
    store
        .append(SessionId::new(99), Message::user("trực tiếp"))
        .await
        .unwrap();
    assert_eq!(store.count(SessionId::new(99)).await.unwrap(), 1);
    assert!(store.summary(SessionId::new(99)).await.unwrap().is_none());
}

/// (Runtime toggle) Hai bản cài đặt phải cùng ngữ nghĩa: override **2 chiều** —
/// cả bật lẫn tắt đều ghi hàng (`enabled` 0/1), thắng seed từ config; vắng mặt =
/// theo seed; sort theo tên ổn định.
#[tokio::test]
async fn tool_override_semantics_match_between_stores() {
    // SQLite: persist qua khởi động lại (mở lại store mới cùng file).
    let dir = TempDir::new().unwrap();
    {
        let store = open(&dir);
        store
            .set_tool_enabled("web_fetch", false, "web:admin")
            .await
            .unwrap();
        store
            .set_tool_enabled("run_shell", false, "web:admin")
            .await
            .unwrap();
        // Tắt lần nữa là idempotent (UPSERT), không lỗi, không trùng.
        store
            .set_tool_enabled("run_shell", false, "web:admin")
            .await
            .unwrap();
        // Bật một tool khác — override 2 chiều: vẫn tạo hàng enabled = 1 (phải
        // thắng được seed khi config khởi động tắt tool này).
        store
            .set_tool_enabled("read_file", true, "web:admin")
            .await
            .unwrap();
    }
    let reopened = open(&dir);
    assert_eq!(
        reopened.tool_overrides().await.unwrap(),
        vec![
            ToolOverride {
                name: "read_file".to_string(),
                enabled: true,
            },
            ToolOverride {
                name: "run_shell".to_string(),
                enabled: false,
            },
            ToolOverride {
                name: "web_fetch".to_string(),
                enabled: false,
            },
        ],
        "sort theo tên, giữ 2 chiều qua khởi động lại"
    );
    reopened
        .set_tool_enabled("web_fetch", true, "web:admin")
        .await
        .unwrap();
    assert_eq!(
        reopened.tool_overrides().await.unwrap(),
        vec![
            ToolOverride {
                name: "read_file".to_string(),
                enabled: true,
            },
            ToolOverride {
                name: "run_shell".to_string(),
                enabled: false,
            },
            ToolOverride {
                name: "web_fetch".to_string(),
                enabled: true,
            },
        ],
        "bật/tắt cập nhật đúng hàng, không xoá"
    );

    // In-memory: cùng ngữ nghĩa.
    let memory = MemoryStore::new();
    assert!(memory.tool_overrides().await.unwrap().is_empty());
    memory
        .set_tool_enabled("mcp__x__y", false, "web:admin")
        .await
        .unwrap();
    assert_eq!(
        memory.tool_overrides().await.unwrap(),
        vec![ToolOverride {
            name: "mcp__x__y".to_string(),
            enabled: false,
        }]
    );
    memory
        .set_tool_enabled("mcp__x__y", true, "web:admin")
        .await
        .unwrap();
    assert_eq!(
        memory.tool_overrides().await.unwrap(),
        vec![ToolOverride {
            name: "mcp__x__y".to_string(),
            enabled: true,
        }]
    );
}

/// K7: web session (đăng nhập UI) — cùng hợp đồng trên cả hai bản: hợp lệ trong
/// cửa sổ expiry, hết hạn là `None`, touch xoá được, delete_all xoá mọi phiên.
#[tokio::test]
async fn web_session_lifecycle_in_both_stores() {
    async fn flow(store: &impl Store) {
        let hash_a = vec![1u8, 2, 3];
        let hash_b = vec![4u8, 5, 6];
        store
            .create_web_session(
                hash_a.clone(),
                "web:admin",
                "2026-11-15T00:00:00Z",
                "2099-01-01T00:00:00Z",
            )
            .await
            .unwrap();
        store
            .create_web_session(
                hash_b.clone(),
                "web:admin",
                "2026-11-15T00:00:00Z",
                "2099-01-01T00:00:00Z",
            )
            .await
            .unwrap();

        let valid = store
            .get_web_session(&hash_a, "2026-11-16T00:00:00Z")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(valid.user_id, "web:admin");
        // Hết hạn (expires_at <= now) phải `None` — cookie hết hạn bị từ chối.
        assert!(
            store
                .get_web_session(&hash_a, "2099-01-01T00:00:00Z")
                .await
                .unwrap()
                .is_none()
        );
        // Hash lạ: `None` và `touch` `false` — không lộ thông tin gì.
        assert!(
            store
                .get_web_session(&[9, 9, 9], "2026-11-16T00:00:00Z")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .touch_web_session(&hash_a, "2026-11-16T00:00:00Z")
                .await
                .unwrap()
        );
        assert!(
            !store
                .touch_web_session(&[9, 9, 9], "2026-11-16T00:00:00Z")
                .await
                .unwrap()
        );

        // Xoá một phiên: lần sau `false`; phiên còn lại vẫn hợp lệ.
        assert!(store.delete_web_session(&hash_a).await.unwrap());
        assert!(!store.delete_web_session(&hash_a).await.unwrap());
        assert!(
            store
                .get_web_session(&hash_b, "2026-11-16T00:00:00Z")
                .await
                .unwrap()
                .is_some()
        );

        // Đổi mật khẩu (M29) gọi delete_all: mọi phiên bị revoke.
        store.delete_all_web_sessions().await.unwrap();
        assert!(
            store
                .get_web_session(&hash_b, "2026-11-16T00:00:00Z")
                .await
                .unwrap()
                .is_none()
        );
    }
    let dir = TempDir::new().unwrap();
    flow(&open(&dir)).await;
    flow(&MemoryStore::new()).await;
}

/// K7: outbox — tin chủ động lỗi phải được retry đúng hợp đồng ở cả hai bản:
/// chỉ trả bản đến hạn, retry tăng attempts + ghi lỗi, complete xoá bản ghi.
#[tokio::test]
async fn outbox_lifecycle_in_both_stores() {
    async fn flow(store: &impl Store) {
        let payload = Outbound {
            session_id: SessionId::new(5),
            message_id: 7,
            text: "tin chủ động 🧸".into(),
            kind: OutboundKind::Notification,
            action: None,
        };
        // Bản A đến hạn ngay; bản B hẹn tương lai — chỉ A được trả.
        let id_a = store
            .enqueue_outbound("telegram", "123", &payload, "2026-11-15T00:00:00Z")
            .await
            .unwrap();
        store
            .enqueue_outbound("telegram", "456", &payload, "2099-01-01T00:00:00Z")
            .await
            .unwrap();

        let due = store.due_outbox("2026-11-16T00:00:00Z", 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id_a);
        assert_eq!(due[0].payload, payload);
        assert_eq!(due[0].attempts, 0);

        // Limit cắt bớt khi có nhiều bản đến hạn hơn limit (limit=0 nghĩa là
        // không giới hạn, nên không dùng 0 để test cắt).
        let id_c = store
            .enqueue_outbound("telegram", "789", &payload, "2026-11-16T00:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            store
                .due_outbox("2026-11-17T00:00:00Z", 1)
                .await
                .unwrap()
                .len(),
            1
        );
        // Cả hai bản đến hạn đều còn nguyên trong store.
        let all_due = store.due_outbox("2026-11-17T00:00:00Z", 10).await.unwrap();
        assert!(all_due.iter().any(|e| e.id == id_a));
        assert!(all_due.iter().any(|e| e.id == id_c));

        // Retry: tăng attempts, đẩy next_attempt_at sang tương lai, ghi lỗi gần nhất.
        store
            .retry_outbox(id_a, "2099-01-01T00:00:00Z", "mạng lỗi: timeout")
            .await
            .unwrap();
        let after_retry = store.due_outbox("2026-11-16T00:00:00Z", 10).await.unwrap();
        assert!(
            after_retry.iter().all(|entry| entry.id != id_a),
            "bản retry chưa đến hạn không được trả lại"
        );
        let due_later = store.due_outbox("2099-01-02T00:00:00Z", 10).await.unwrap();
        let retried = due_later.iter().find(|entry| entry.id == id_a).unwrap();
        assert_eq!(retried.attempts, 1);
        assert_eq!(retried.last_error, "mạng lỗi: timeout");

        // Gửi thành công: complete xoá — không còn trong danh sách đến hạn.
        store.complete_outbox(id_a).await.unwrap();
        assert!(
            store
                .due_outbox("2099-01-02T00:00:00Z", 10)
                .await
                .unwrap()
                .iter()
                .all(|entry| entry.id != id_a)
        );
    }
    let dir = TempDir::new().unwrap();
    flow(&open(&dir)).await;
    flow(&MemoryStore::new()).await;
}
