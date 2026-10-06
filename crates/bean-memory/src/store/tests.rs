//! Test dùng **chung cho cả hai** bản cài đặt ([`MemoryStore`](super::MemoryStore) và
//! [`SqliteStore`](super::SqliteStore)).
//!
//! # Vì sao tách ra file riêng
//!
//! Trước đây 579 dòng test nằm cuối `store.rs`, chồng lên các module thật. Tách ra đây
//! giữ file sản phẩm chỉ còn code chạy thật, và khiến một điểm trở nên hiển nhiên:
//!
//! # Ràng buộc bất biến
//!
//! Mọi hàm bên dưới chạy **trên cả hai** bản cài đặt. Đây là cách duy nhất giữ được
//! ràng buộc "hai bản phải cùng ngữ nghĩa" — nếu chỉ test `SqliteStore`, một lỗi chỉ xảy
//! ra ở `MemoryStore` (dùng cho **mọi** test vòng lặp agent) sẽ không bao giờ bị phát
//! hiện. Thêm hàm kiểm tra mới mà quên gọi trên cả hai bản là thất lạc.

// Test thì được phép `unwrap` — nguyên tắc "không panic trong code sản phẩm"
// (agents.md mục 0.8) không áp dụng cho test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use bean_llm::{FakeProvider, LlmProvider};
use bean_types::{Config, LlmResponse, Message, Outbound, OutboundKind, Role, SessionId, ToolCall};
use tempfile::TempDir;

use super::sqlite::migration::configure;
use super::sqlite::schema::SCHEMA_SQL;
use super::{MemorySearchHit, MemorySource, MemoryStore, SqliteStore, Store};
use crate::safe_cut::check_no_orphan_result;

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("bean.db")
}

fn open(dir: &TempDir) -> SqliteStore {
    SqliteStore::open(&db_path(dir)).unwrap()
}

/// Một lượt "đọc file" hoàn chỉnh: user → assistant(gọi tool) → tool result.
fn tool_round(index: usize, filler: &str) -> Vec<Message> {
    let id = format!("c{index}");
    vec![
        Message::user(format!("{filler} câu hỏi {index}")),
        Message::assistant(
            None,
            vec![ToolCall::new(
                id.clone(),
                "probe",
                serde_json::json!({ "n": index }),
            )],
        ),
        Message::tool(id, format!("{filler} kết quả {index}")),
    ]
}

fn texts(hits: &[MemorySearchHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.text.as_str()).collect()
}

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
    let hits = reopened.memory_search("Vinh").await.unwrap();
    assert!(hits.iter().any(|h| h.source == MemorySource::Memories));
}

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

/// Outbox bền vững: ghi lỗi, tăng attempts, đổi lịch rồi xoá khi gửi thành công.
#[tokio::test]
async fn sqlite_outbox_retry_lifecycle_survives_restart() {
    let dir = TempDir::new().unwrap();
    let payload = Outbound {
        session_id: SessionId::new(7),
        message_id: 42,
        text: "tin chủ động 🦀".into(),
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
    // 6 = đã chạy tới migration `mcp_clients` (M25).
    assert_eq!(version, 6);
}

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

/// `limit` giữ **message cuối**, `before_seq` phân trang ngược, `seq` bắt đầu từ 1.
#[tokio::test]
async fn history_honours_limit_and_paging() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    for index in 0..5 {
        store
            .append(session, Message::user(format!("tin {index}")))
            .await
            .unwrap();
    }
    assert_eq!(store.history(session, None, 0).await.unwrap().len(), 5);
    let last_two = store.history(session, None, 2).await.unwrap();
    assert_eq!(last_two.len(), 2);
    assert_eq!(last_two[0].text.as_deref(), Some("tin 3"));
    assert_eq!(last_two[1].text.as_deref(), Some("tin 4"));
    let page = store.history(session, Some(3), 2).await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].text.as_deref(), Some("tin 0"));
    assert_eq!(page[1].text.as_deref(), Some("tin 1"));
    let stored = store.list_messages(session, None, 0).await.unwrap();
    assert_eq!(stored[0].seq, 1);
    assert_eq!(stored[4].seq, 5);
}

/// FTS5 tìm được cả `memories` lẫn `messages` (agents.md mục 8.4).
#[tokio::test]
async fn fts5_finds_memories_and_messages() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    store
        .memory_save("Người dùng thích báo cáo tài chính quý 4", "sở thích")
        .await
        .unwrap();
    store
        .append(session, Message::user("lập báo cáo tài chính quý 4"))
        .await
        .unwrap();
    store
        .append(session, Message::user("thời tiết hôm nay thế nào"))
        .await
        .unwrap();

    let hits = store.memory_search("báo cáo").await.unwrap();
    assert!(hits.iter().any(|h| h.source == MemorySource::Memories));
    assert!(
        hits.iter()
            .any(|h| matches!(h.source, MemorySource::Message { .. })),
        "phải tìm được trong lịch sử hội thoại: {:?}",
        texts(&hits)
    );
    assert!(hits.iter().all(|h| h.text.contains("báo")));
    assert!(store.memory_search("thời tiết").await.unwrap().len() == 1);
}

/// Điểm chuẩn hoá theo nguồn: ghi nhớ dài hạn không bị "chìm" chỉ vì bảng có ít dòng
/// (BM25 của bảng 1 dòng có IDF ≈ 0).
#[tokio::test]
async fn fts5_normalizes_scores_per_source() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    store.memory_save("hồ sơ của Vinh", "hồ sơ").await.unwrap();
    store
        .append(session, Message::user("Vinh hỏi về hồ sơ"))
        .await
        .unwrap();

    let hits = store.memory_search("hồ sơ Vinh").await.unwrap();
    let best_memory = hits
        .iter()
        .find(|hit| hit.source == MemorySource::Memories)
        .unwrap();
    let best_message = hits
        .iter()
        .find(|hit| matches!(hit.source, MemorySource::Message { .. }))
        .unwrap();
    assert_eq!(best_memory.score, 1.0);
    assert_eq!(best_message.score, 1.0);
    // Bằng điểm ⇒ ghi nhớ dài hạn đứng trước (xem ghi chú trong `memory_search`).
    assert_eq!(hits[0].source, MemorySource::Memories);
}

/// BM25: term xuất hiện nhiều lần trong cùng độ dài văn bản phải xếp trước.
#[tokio::test]
async fn fts5_ranks_higher_term_frequency_first() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    store
        .memory_save("alpha beta gamma delta", "a")
        .await
        .unwrap();
    store
        .memory_save("alpha alpha alpha alpha alpha", "b")
        .await
        .unwrap();
    let hits = store.memory_search("alpha").await.unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].text, "alpha alpha alpha alpha alpha");
    assert!(hits[0].score > hits[1].score, "điểm: {:?}", hits);
}

/// Tiếng Việt: tìm có dấu và không dấu đều phải khớp (tokenizer `remove_diacritics 2`).
#[tokio::test]
async fn fts5_matches_vietnamese_with_or_without_diacritics() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    store
        .memory_save("Báo cáo tài chính quý 4 cần gửi trước thứ Sáu", "công việc")
        .await
        .unwrap();
    for query in ["báo cáo", "bao cao", "BÁO CÁO", "tai chinh", "thu sau"] {
        let hits = store.memory_search(query).await.unwrap();
        assert!(!hits.is_empty(), "không tìm thấy với query `{query}`");
    }
}

/// Query của model là đầu vào không tin cậy: mọi ký tự điều khiển FTS phải bị vô hiệu.
#[tokio::test]
async fn fts_search_tolerates_special_characters() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    store
        .memory_save("ghi nhớ về dự án Bean", "test")
        .await
        .unwrap();
    for query in [
        "",
        "   ",
        "\"",
        "*",
        "( )",
        "NEAR(",
        "AND OR NOT",
        "^dự án$",
        "-",
        "dự án\" OR 1=1 --",
        "😀 emoji",
        "café:test*",
    ] {
        // Không được panic hay trả lỗi cú pháp FTS.
        store.memory_search(query).await.unwrap();
    }
    assert!(!store.memory_search("dự án").await.unwrap().is_empty());
    assert!(store.memory_search("😀").await.unwrap().is_empty());
}

/// Trigger FTS phải đồng bộ khi xoá (`delete_before`, `clear`).
#[tokio::test]
async fn delete_and_clear_keep_fts_in_sync() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    store
        .append(session, Message::user("chủ đề zebra"))
        .await
        .unwrap();
    store
        .append(session, Message::user("chủ đề unicorn"))
        .await
        .unwrap();
    assert!(!store.memory_search("zebra").await.unwrap().is_empty());

    store.delete_before(session, 2).await.unwrap();
    assert!(
        store.memory_search("zebra").await.unwrap().is_empty(),
        "FTS chưa đồng bộ sau khi xoá"
    );
    store.clear(session).await.unwrap();
    assert!(store.memory_search("unicorn").await.unwrap().is_empty());
    assert_eq!(store.count(session).await.unwrap(), 0);
}

/// Compaction (mục 8.3): tóm tắt phần cũ, giữ phần gần đây, **không** tách cặp tool.
#[tokio::test]
async fn compaction_keeps_tool_pairs_and_saves_summary() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    let filler = "dữ liệu khá dài để đo ngân sách token ".repeat(5);
    let filler = filler.as_str();
    for index in 0..12 {
        for message in tool_round(index, filler) {
            store.append(session, message).await.unwrap();
        }
    }
    assert_eq!(store.count(session).await.unwrap(), 36);

    let mut config = Config::default();
    config.agent.context_budget_tokens = 1_000;
    let provider = FakeProvider::new(vec![LlmResponse::text_only("TÓM TẮT: đang làm việc X")]);
    let llm: &dyn LlmProvider = &provider;
    store.compact(session, llm, &config).await.unwrap();

    assert_eq!(
        store.summary(session).await.unwrap().as_deref(),
        Some("TÓM TẮT: đang làm việc X")
    );
    // Giữ 20 message cuối ⇒ bắt đầu ở `user` của lượt thứ 7 (index 18).
    assert_eq!(store.count(session).await.unwrap(), 18);
    let history = store.history(session, None, 0).await.unwrap();
    assert_eq!(history.first().map(|m| m.role), Some(Role::User));
    assert!(
        check_no_orphan_result(&history, 0),
        "compaction đã tách cặp assistant/tool"
    );
    assert_eq!(
        history.last().unwrap().tool_call_id.as_deref(),
        Some("c11"),
        "phải giữ nguyên lượt gần nhất"
    );
    // Nội dung đã bị xoá không còn trong FTS.
    assert!(store.memory_search("kết quả 0").await.unwrap().is_empty());
}

/// Dưới ngưỡng 70% thì **không** gọi LLM và không đụng vào lịch sử.
#[tokio::test]
async fn compaction_skipped_when_under_budget() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    store
        .append(session, Message::user("một câu ngắn"))
        .await
        .unwrap();
    let config = Config::default();
    // Có sẵn câu trả lời ⇒ nếu bị gọi thì `summary` sẽ khác `None`.
    let provider = FakeProvider::new(vec![LlmResponse::text_only("không được gọi")]);
    let llm: &dyn LlmProvider = &provider;
    store.compact(session, llm, &config).await.unwrap();
    assert!(store.summary(session).await.unwrap().is_none());
    assert_eq!(store.count(session).await.unwrap(), 1);
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
    let hits = store.memory_search("zeta").await.unwrap();
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
