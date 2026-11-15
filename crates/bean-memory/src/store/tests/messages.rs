//! Lịch sử tin nhắn: phân trang `limit`/`before_seq` và compaction (mục 8.3)
//! với `FakeProvider` — tóm tắt phần cũ, giữ cặp tool_use/tool_result nguyên.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bean_llm::{FakeProvider, LlmProvider};
use bean_types::{Config, LlmResponse, Message, Role};
use tempfile::TempDir;

use super::{open, tool_round};
use crate::safe_cut::check_no_orphan_result;
use crate::store::Store;
// Method gọi trên struct cụ thể cần sub-trait tương ứng trong scope.
use crate::traits::{MemoryStore as _, MessageStore, SessionStore};

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
    assert!(
        store
            .memory_search("kết quả 0", None)
            .await
            .unwrap()
            .is_empty()
    );
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
