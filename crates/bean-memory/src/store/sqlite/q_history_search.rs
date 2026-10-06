//! Doc lich su theo phan trang + **FTS5/BM25** cho `memory_search` (muc 8.1).

use rusqlite::{Connection, params};

use super::migration::internal;
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
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let Some(match_expr) = sanitize_fts_query(query) else {
        return Ok(Vec::new());
    };
    let limit = MEMORY_SEARCH_LIMIT as i64;
    let mut memories = search_memories(conn, &match_expr, limit)?;
    let mut messages = search_messages(conn, &match_expr, limit)?;
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
pub(super) fn search_messages(
    conn: &Connection,
    match_expr: &str,
    limit: i64,
) -> Result<Vec<MemorySearchHit>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT m.text_for_search, m.session_id, bm25(messages_fts) \
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?1 ORDER BY 3 LIMIT ?2",
        )
        .map_err(internal)?;
    let rows = stmt
        .query_map(params![match_expr, limit], |row| {
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

/// Làm sạch query của người dùng/model trước khi đưa vào `MATCH` (agents.md mục 8.4).
///
/// Giữ lại ký tự chữ-số (kể cả dấu tiếng Việt) và `_`, bọc mỗi từ khoá trong `\"`.
/// Trả `None` khi không còn từ khoá nào — khi đó tìm kiếm trả về rỗng thay vì lỗi cú pháp.
pub(super) fn sanitize_fts_query(query: &str) -> Option<String> {
    let mut tokens: Vec<String> = Vec::new();
    for raw in query.split_whitespace() {
        let cleaned: String = raw
            .chars()
            .filter(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        tokens.push(format!("\"{cleaned}\""));
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}
