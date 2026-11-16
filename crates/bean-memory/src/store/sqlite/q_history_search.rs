//! Doc lich su theo phan trang + **FTS5/BM25** cho `memory_search` (muc 8.1).

use rusqlite::{Connection, params};

use super::migration::internal;
use super::q_fts_sanitize::sanitize_fts_query;
use super::q_session::decode_message;
use crate::store::shared::{sql_limit, truncate_chars};
use crate::store::types::{MemorySearchHit, MemorySource, StoreError, StoredMessage};
use crate::store::{MEMORY_HIT_PREVIEW_CHARS, MEMORY_SEARCH_LIMIT};
use bean_types::{Message, SessionId};

pub(super) fn load_history(
    conn: &Connection,
    session: SessionId,
    before_seq: Option<u64>,
    limit: usize,
) -> Result<Vec<Message>, StoreError> {
    let before = before_seq.map(|seq| seq as i64);
    let mut stmt = conn
        .prepare(
            "SELECT content_json FROM (\
               SELECT seq, content_json FROM messages \
               WHERE session_id = ?1 AND (?2 IS NULL OR seq < ?2) \
               ORDER BY seq DESC LIMIT ?3\
             ) ORDER BY seq ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session.get(), before, sql_limit(limit)], |row| {
            row.get::<_, String>(0)
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let json = row.map_err(internal)?;
        out.push(decode_message(&json)?);
    }
    Ok(out)
}

pub(super) fn list_messages(
    conn: &Connection,
    session: SessionId,
    before_seq: Option<u64>,
    limit: usize,
) -> Result<Vec<StoredMessage>, StoreError> {
    let before = before_seq.map(|seq| seq as i64);
    let mut stmt = conn
        .prepare(
            "SELECT seq, content_json FROM (\
               SELECT seq, content_json FROM messages \
               WHERE session_id = ?1 AND (?2 IS NULL OR seq < ?2) \
               ORDER BY seq DESC LIMIT ?3\
             ) ORDER BY seq ASC",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![session.get(), before, sql_limit(limit)], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, json) = row.map_err(internal)?;
        out.push(StoredMessage {
            seq: seq.max(0) as u64,
            message: decode_message(&json)?,
        });
    }
    Ok(out)
}

pub(super) fn delete_before(
    conn: &Connection,
    session: SessionId,
    before_seq: u64,
) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM messages WHERE session_id = ?1 AND seq < ?2",
        params![session.get(), before_seq as i64],
    )
    .map_err(internal)?;
    Ok(())
}

/// Tìm kiếm BM25 trong `memories` **và** `messages` (agents.md mục 8.4).
///
/// `MATCH` không bao giờ nhận query thô của model: mọi từ khoá được bọc trong dấu ngoặc
/// kép (`sanitize_fts_query`) nên cú pháp FTS không thể bị phá.
///
/// Điểm BM25 phụ thuộc **kích thước bảng** (bảng ít dòng ⇒ IDF ≈ 0 ⇒ điểm rất nhỏ), nên
/// điểm được chuẩn hoá **trong từng nguồn** trước khi trộn: `1.0` là kết quả liên quan
/// nhất của nguồn đó. Nhờ vậy một ghi nhớ dài hạn không bị "chìm" chỉ vì bảng `memories`
/// có ít dòng hơn `messages`.
pub(super) fn memory_search(
    conn: &Connection,
    query: &str,
    exclude_current_run_of: Option<SessionId>,
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let Some(match_expr) = sanitize_fts_query(query) else {
        return Ok(Vec::new());
    };
    let limit = MEMORY_SEARCH_LIMIT as i64;
    let run_cut = match exclude_current_run_of {
        Some(session) => current_run_start_seq(conn, session)?.map(|cut| (session.get(), cut)),
        None => None,
    };
    let mut memories = search_memories(conn, &match_expr, limit)?;
    let mut messages = search_messages(conn, &match_expr, limit, run_cut)?;
    normalize_scores(&mut memories);
    normalize_scores(&mut messages);

    // `sort_by` ổn định ⇒ hai kết quả cùng điểm (mỗi nguồn đều có điểm 1.0) giữ thứ tự
    // chèn: ghi nhớ dài hạn đứng trước, rồi tới lịch sử hội thoại.
    let mut hits = memories;
    hits.extend(messages);
    hits.sort_by(|left, right| right.score.total_cmp(&left.score));
    hits.truncate(MEMORY_SEARCH_LIMIT);
    Ok(hits)
}

/// Seq của message `user` cuối cùng trong session — ranh giới mở đầu lượt hiện tại.
///
/// `None` khi session chưa có message user nào: khi đó không có lượt nào đang chạy,
/// không loại gì cả (không dùng COALESCE 0 vì sẽ loại nhầm toàn bộ lịch sử).
pub(super) fn current_run_start_seq(
    conn: &Connection,
    session: SessionId,
) -> Result<Option<i64>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT MAX(seq) FROM messages WHERE session_id = ?1 AND role = 'user'")
        .map_err(internal)?;
    let mut rows = stmt
        .query_map(params![session.get()], |row| row.get::<_, Option<i64>>(0))
        .map_err(internal)?;
    let first = rows.next().transpose().map_err(internal)?;
    Ok(first.flatten())
}

/// Tìm trong bảng `memories` (chỉ mục FTS5 external content).
pub(super) fn search_memories(
    conn: &Connection,
    match_expr: &str,
    limit: i64,
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.text, bm25(memories_fts) FROM memories_fts \
             JOIN memories m ON m.id = memories_fts.rowid \
             WHERE memories_fts MATCH ?1 ORDER BY 2 LIMIT ?2",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![match_expr, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (text, score) = row.map_err(internal)?;
        out.push(MemorySearchHit {
            score: bm25_to_score(score),
            text,
            source: MemorySource::Memories,
        });
    }
    Ok(out)
}

/// Tìm trong lịch sử hội thoại (`text_for_search` gồm cả tên/đối số tool).
///
/// `run_cut` — khi có (`Some(session_id_thô, cut)`), loại message của session đó có
/// `seq > cut`: đó là các message của lượt đang chạy (đối số tool, kết quả tool,
/// assistant trung gian) — K5. Chỉ message của session chỉ định bị ảnh hưởng.
pub(super) fn search_messages(
    conn: &Connection,
    match_expr: &str,
    limit: i64,
    run_cut: Option<(i64, i64)>,
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.text_for_search, m.session_id, bm25(messages_fts) \
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?1 \
               AND (?2 IS NULL OR m.session_id != ?2 OR m.seq <= ?3) \
             ORDER BY 3 LIMIT ?4",
        )
        .map_err(internal)?;
    let (cut_session, cut_seq) = run_cut.unzip();
    let rows = stmt
        .query_map(params![match_expr, cut_session, cut_seq, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        let (text, session_id, score) = row.map_err(internal)?;
        out.push(MemorySearchHit {
            score: bm25_to_score(score),
            text: truncate_chars(&text, MEMORY_HIT_PREVIEW_CHARS),
            source: MemorySource::Message { session_id },
        });
    }
    Ok(out)
}

/// Chia mọi điểm cho điểm cao nhất của **cùng nguồn** ⇒ miền giá trị `[0, 1]`.
pub(super) fn normalize_scores(hits: &mut [MemorySearchHit]) {
    let best = hits.iter().map(|hit| hit.score).fold(0.0_f32, f32::max);
    if best > 0.0 {
        for hit in hits.iter_mut() {
            hit.score /= best;
        }
    }
}
/// `bm25()` trả giá trị **âm** (nhỏ hơn = liên quan hơn) ⇒ đảo dấu cho dễ đọc.
pub(super) fn bm25_to_score(raw: f64) -> f32 {
    (-raw) as f32
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use rusqlite::Connection;

    use super::super::migration::{configure, run_migration};
    use super::{memory_search, sanitize_fts_query, search_memories, search_messages};
    use crate::store::types::MemorySource;

    /// (K8) Đặc ta hành vi chuẩn hoá điểm **theo nguồn**: thứ tự giữa `memories` và
    /// `messages` là quy ước (bằng điểm ⇒ ghi nhớ trước), không phải so điểm BM25 gốc
    /// giữa hai nguồn (xem doc comment `memory_search`).
    ///
    /// Dựng dữ liệu để message liên quan hơn hẳn ghi nhớ về điểm gốc — IDF của bảng
    /// `messages` cao hơn vì có nhiều dòng, TF của message gấp 5 lần. Nếu ai đó bỏ
    /// chuẩn hoá per-source để "sửa K8" mà không có thiết kế thay thế có chủ đích
    /// (ví dụ RRF), test này đỏ ngay: khi đó message phải xếp trước ghi nhớ.
    #[test]
    fn k8_chuan_hoa_theo_nguon_giu_ghi_nho_dung_truoc() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        run_migration(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (id, channel, chat_id, title, archived, summary, created_at, updated_at)
             VALUES (1, 'cli', 'local', '', 0, '', '2026-11-16T00:00:00Z', '2026-11-16T00:00:00Z');
             INSERT INTO messages (session_id, seq, role, content_json, text_for_search, created_at) VALUES
               (1, 1, 'user', '{}', 'chủ đề khác hoàn toàn', '2026-11-16T00:00:00Z'),
               (1, 2, 'user', '{}', 'một chủ đề khác nữa', '2026-11-16T00:00:00Z'),
               (1, 3, 'user', '{}', 'vinh vinh vinh vinh vinh', '2026-11-16T00:00:00Z'),
               (1, 4, 'user', '{}', 'lại một chủ đề khác', '2026-11-16T00:00:00Z');
             INSERT INTO memories (text, tags, created_at) VALUES
               ('hồ sơ người dùng: Vinh thích cà phê sữa đá, hay làm việc vào buổi tối, và nhiều chi tiết khác khiến văn bản này dài hơn hẳn các message', '', '2026-11-16T00:00:00Z');",
        )
        .unwrap();

        let expr = sanitize_fts_query("vinh").unwrap();

        // Điểm BM25 gốc (trước chuẩn hoá): message phải liên quan hơn hẳn ghi nhớ —
        // đây là giả định của kịch bản; vi phạm nghĩa là test mất ý nghĩa.
        let raw_memories = search_memories(&conn, &expr, 20).unwrap();
        let raw_messages = search_messages(&conn, &expr, 20, None).unwrap();
        assert_eq!(raw_memories.len(), 1);
        assert_eq!(raw_messages.len(), 1);
        assert!(
            raw_messages[0].score > raw_memories[0].score,
            "giả định K8: message phải mạnh hơn hẳn về điểm gốc (message {}, memory {})",
            raw_messages[0].score,
            raw_memories[0].score
        );

        // Qua `memory_search`: chuẩn hoá per-source làm cả hai đỉnh đều 1.0 và ghi nhớ
        // đứng trước dù điểm gốc yếu hơn — đúng quy ước ghi trong doc comment.
        let hits = memory_search(&conn, "vinh", None).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].source, MemorySource::Memories);
        assert_eq!(hits[0].score, 1.0);
        assert_eq!(hits[1].source, MemorySource::Message { session_id: 1 });
        assert_eq!(hits[1].score, 1.0);
    }
}
