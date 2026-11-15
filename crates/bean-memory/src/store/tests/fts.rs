//! FTS5 và tìm kiếm bộ nhớ (agents.md mục 8.4): tìm cả `memories` lẫn
//! `messages`, loại message của lượt đang chạy (K5), chuẩn hoá điểm theo nguồn,
//! tiếng Việt có/không dấu, ký tự đặc biệt, đồng bộ trigger khi xoá.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bean_types::{Message, ToolCall};
use tempfile::TempDir;

use super::{open, texts};
use crate::store::{MemorySource, MemoryStore, Store};
// Method gọi trên struct cụ thể cần sub-trait tương ứng trong scope.
use crate::traits::{MemoryStore as _, MessageStore, SessionStore};

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

    let hits = store.memory_search("báo cáo", None).await.unwrap();
    assert!(hits.iter().any(|h| h.source == MemorySource::Memories));
    assert!(
        hits.iter()
            .any(|h| matches!(h.source, MemorySource::Message { .. })),
        "phải tìm được trong lịch sử hội thoại: {:?}",
        texts(&hits)
    );
    assert!(hits.iter().all(|h| h.text.contains("báo")));
    assert!(store.memory_search("thời tiết", None).await.unwrap().len() == 1);
}

/// (K5) `memory_search` gọi từ tool trong lượt đang chạy phải loại mọi message
/// sau message `user` cuối cùng: đối số tool — kể cả chính query vừa gửi — đã được
/// ghi DB trước khi tool chạy (mục 6) và thường chiếm top kết quả, gây nhiễu cho
/// model. Lịch sử các lượt trước và session khác vẫn phải tìm thấy. Cắt theo
/// `MAX(seq) WHERE role = 'user'` nên message `user` mở đầu lượt vẫn tìm thấy.
#[tokio::test]
async fn memory_search_bo_message_cua_luot_dang_chay() {
    let dir = TempDir::new().unwrap();
    let store = open(&dir);
    let session = store.ensure_session("cli", "local", "").await.unwrap();
    let other = store.ensure_session("web", "uuid-khac", "").await.unwrap();

    // Lượt TRƯỚC của cùng session: nội dung này vẫn phải tìm thấy.
    store
        .append(session, Message::user("lập báo cáo tài chính quý 4"))
        .await
        .unwrap();
    // Session khác: không thuộc lượt hiện tại, vẫn phải tìm thấy.
    store
        .append(other, Message::user("báo cáo tài chính của session khác"))
        .await
        .unwrap();

    // Lượt HIỆN TẠI: user mở đầu, model gọi memory_search — assistant(tool_calls)
    // chứa query trong đối số và tool result ghi DB trước khi tool chạy (mục 6).
    store
        .append(session, Message::user("tìm lại báo cáo"))
        .await
        .unwrap();
    store
        .append(
            session,
            Message::assistant(
                None,
                vec![ToolCall::new(
                    "c1",
                    "memory_search",
                    serde_json::json!({ "query": "báo cáo tài chính" }),
                )],
            ),
        )
        .await
        .unwrap();
    store
        .append(session, Message::tool("c1", "kết quả zebratrunggian"))
        .await
        .unwrap();

    // Gọi như tool (Some(session)): mọi Message hit phải đến từ session khác —
    // message của lượt đang chạy (tool call + result) bị loại, còn message user
    // của lượt trước (seq trước user cuối) vẫn tìm thấy.
    let hits = store
        .memory_search("báo cáo tài chính", Some(session))
        .await
        .unwrap();
    assert!(
        hits.iter()
            .any(|hit| matches!(hit.source, MemorySource::Message { session_id } if session_id == session.get())),
        "phải giữ lịch sử lượt trước (cùng session): {:?}",
        texts(&hits)
    );
    assert!(
        hits.iter()
            .any(|hit| matches!(hit.source, MemorySource::Message { session_id } if session_id == other.get())),
        "phải giữ lịch sử session khác: {:?}",
        texts(&hits)
    );
    // Message cùng session được trả chỉ có thể là lịch sử lượt trước — không
    // được có assistant(tool_calls) hay tool result của lượt đang chạy.
    for hit in &hits {
        if let MemorySource::Message { session_id } = &hit.source
            && *session_id == session.get()
        {
            assert!(
                hit.text.contains("lập báo cáo tài chính quý 4"),
                "chỉ user message lượt trước được trả: {hit:?}"
            );
        }
    }
    // Token duy nhất chỉ nằm trong tool result của lượt đang chạy: bị loại.
    assert!(
        store
            .memory_search("zebratrunggian", Some(session))
            .await
            .unwrap()
            .is_empty()
    );
    // Gọi ngoài lượt (None): tìm thấy cả message của lượt vừa rồi.
    assert!(
        !store
            .memory_search("zebratrunggian", None)
            .await
            .unwrap()
            .is_empty()
    );
    // Session chưa từng có message user: không loại gì, không lỗi.
    let fresh = store.ensure_session("web", "uuid-moi", "").await.unwrap();
    assert!(
        !store
            .memory_search("zebratrunggian", Some(fresh))
            .await
            .unwrap()
            .is_empty()
    );
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

    let hits = store.memory_search("hồ sơ Vinh", None).await.unwrap();
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
    let hits = store.memory_search("alpha", None).await.unwrap();
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
        let hits = store.memory_search(query, None).await.unwrap();
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
        store.memory_search(query, None).await.unwrap();
    }
    assert!(!store.memory_search("dự án", None).await.unwrap().is_empty());
    assert!(store.memory_search("😀", None).await.unwrap().is_empty());
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
    assert!(!store.memory_search("zebra", None).await.unwrap().is_empty());

    store.delete_before(session, 2).await.unwrap();
    assert!(
        store.memory_search("zebra", None).await.unwrap().is_empty(),
        "FTS chưa đồng bộ sau khi xoá"
    );
    store.clear(session).await.unwrap();
    assert!(
        store
            .memory_search("unicorn", None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.count(session).await.unwrap(), 0);
}

/// K6: `clear` phải xoá cả summary — summary mô tả lịch sử vừa mất, giữ lại là
/// context sai lệch. Chạy trên **cả hai** bản cài đặt để giữ ngữ nghĩa đồng bộ (K7).
#[tokio::test]
async fn clear_resets_summary_in_both_stores() {
    async fn flow(store: &impl Store) {
        let session = store.ensure_session("cli", "local", "").await.unwrap();
        store
            .append(session, Message::user("lịch sử sẽ bị xoá"))
            .await
            .unwrap();
        store
            .save_summary(session, "tóm tắt của lịch sử đã mất")
            .await
            .unwrap();
        assert_eq!(
            store.summary(session).await.unwrap().as_deref(),
            Some("tóm tắt của lịch sử đã mất")
        );
        store.clear(session).await.unwrap();
        assert!(
            store.summary(session).await.unwrap().is_none(),
            "summary phải bị reset cùng message (K6)"
        );
        assert_eq!(store.count(session).await.unwrap(), 0);
    }
    let dir = TempDir::new().unwrap();
    flow(&open(&dir)).await;
    flow(&MemoryStore::new()).await;
}
