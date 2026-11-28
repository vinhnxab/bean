//! Khôi phục run bị ngắt sau khi Bean tắt đột ngột (crash, OOM, restart hệ thống).
//!
//! Vấn đề (agents.md mục 6, 22.1): message được ghi DB **ngay khi phát sinh**, nên
//! một run chết giữa chừng để lại đuôi lịch sử **thiếu tool result** — message
//! `assistant` mang `tool_calls` mà không có `tool` result tương ứng. Lần gọi API
//! kế tiếp trên phiên đó bị provider từ chối 400 vì cặp `tool_use`/`tool_result`
//! vỡ — đúng lớp lỗi phổ biến nhất mục 22.1.
//!
//! Cách sửa: lúc `serve` khởi động (trước khi Router nhận run mới), quét **đuôi**
//! mỗi phiên; với mỗi `tool_call` chưa được trả lời trong khối tool cuối cùng,
//! chèn một message `tool` tổng hợp `is_error = true` — cùng ngữ nghĩa với rule
//! "huỷ giữa chừng vẫn ghi result `[bị người dùng huỷ]`" (mục 6).
//!
//! Vì sao chỉ sửa đuôi: `append` ghi DB đồng bộ theo thứ tự phát sinh, nên một
//! crash chỉ có thể cắt phần cuối. Không bao giờ tồn tại hai `assistant` dang dở
//! phân tán trong một phiên — nếu có, đó là dữ liệu hỏng vì lý do khác và phải để
//! nguyên để điều tra, không "vá mù".

use std::path::Path;

use bean_types::{Message, ToolCall};
use rusqlite::Connection;

use super::migration::{configure, internal, run_migration};
use crate::store::shared::now_rfc3339;
use crate::store::types::StoreError;

/// Nội dung tool result tổng hợp cho `tool_call` chưa kịp trả lời khi tiến trình chết.
pub const INTERRUPTED_NOTE: &str =
    "[bị ngắt giữa chừng: Bean vừa khởi động lại trong lúc tool đang chạy]";

/// Quét toàn bộ phiên trong DB, chèn tool result tổng hợp cho đuôi dang dở.
///
/// Trả về số message đã chèn. Chạy **một lần lúc boot**, trên connection riêng —
/// SQLite WAL cho phép connection này ghi trong khi worker chưa nhận traffic;
/// xong trước khi `serve` mở cổng nên không có tranh chấp writer.
///
/// # Errors
/// [`StoreError::Internal`] khi mở DB hay ghi thất bại.
pub fn repair_interrupted_runs(path: &Path) -> Result<usize, StoreError> {
    let mut conn = Connection::open(path).map_err(internal)?;
    configure(&conn)?;
    run_migration(&conn)?;
    let tx = conn.transaction().map_err(internal)?;
    let session_ids: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM sessions").map_err(internal)?;
        let rows = stmt.query_map([], |row| row.get(0)).map_err(internal)?;
        rows.collect::<Result<Vec<i64>, _>>().map_err(internal)?
    };
    let mut repaired = 0usize;
    for session_id in session_ids {
        repaired += repair_session_tail(&tx, session_id)?;
    }
    tx.commit().map_err(internal)?;
    Ok(repaired)
}

/// Sửa đuôi một phiên: trả về số tool result tổng hợp đã chèn.
fn repair_session_tail(
    tx: &rusqlite::Transaction<'_>,
    session_id: i64,
) -> Result<usize, StoreError> {
    let mut stmt = tx
        .prepare(
            "SELECT seq, role, content_json FROM messages
             WHERE session_id = ?1 ORDER BY seq DESC",
        )
        .map_err(internal)?;
    let mut rows = stmt.query([session_id]).map_err(internal)?;
    // Đi ngược từ cuối: gom id đã được trả lời trong **khối tool trailing**, rồi
    // dừng ở dòng không phải `tool` đầu tiên — đó là ứng viên `assistant` dang dở
    // (hoặc `user`/`assistant` thường ⇒ phiên sạch).
    let mut answered: Vec<String> = Vec::new();
    let mut dangling: Option<Vec<ToolCall>> = None;
    while let Some(row) = rows.next().map_err(internal)? {
        let role: String = row.get(1).map_err(internal)?;
        if role == "tool" {
            let content: String = row.get(2).map_err(internal)?;
            if let Ok(msg) = serde_json::from_str::<Message>(&content)
                && let Some(id) = msg.tool_call_id
            {
                answered.push(id);
            }
            continue;
        }
        if role == "assistant" {
            let content: String = row.get(2).map_err(internal)?;
            if let Ok(msg) = serde_json::from_str::<Message>(&content)
                && !msg.tool_calls.is_empty()
            {
                dangling = Some(msg.tool_calls);
            }
        }
        break;
    }
    drop(rows);
    drop(stmt);

    let Some(calls) = dangling else {
        return Ok(0);
    };
    let missing: Vec<&ToolCall> = calls
        .iter()
        .filter(|call| !answered.contains(&call.id))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }

    let now = now_rfc3339();
    let mut inserted = 0usize;
    for call in missing {
        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .map_err(internal)?;
        let mut msg = Message::tool(call.id.clone(), INTERRUPTED_NOTE);
        msg.is_error = true;
        let content_json = serde_json::to_string(&msg)
            .map_err(|e| StoreError::Internal(format!("serialize message: {e}")))?;
        tx.execute(
            "INSERT INTO messages (session_id, seq, role, content_json, text_for_search, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![session_id, seq, "tool", content_json, INTERRUPTED_NOTE, now],
        )
        .map_err(internal)?;
        inserted += 1;
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use rusqlite::params;

    fn open_mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        run_migration(&conn).unwrap();
        conn
    }

    fn insert_session(conn: &Connection, id: i64) {
        conn.execute(
            "INSERT INTO sessions (id, channel, chat_id, title, archived, summary, created_at, updated_at)
             VALUES (?1, 'cli', 'local', '', 0, '', '2026-11-27T00:00:00Z', '2026-11-27T00:00:00Z')",
            [id],
        )
        .unwrap();
    }

    fn insert_msg(conn: &Connection, session_id: i64, seq: i64, role: &str, msg: &Message) {
        conn.execute(
            "INSERT INTO messages (session_id, seq, role, content_json, text_for_search, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '2026-11-27T00:00:00Z')",
            params![session_id, seq, role, serde_json::to_string(msg).unwrap(), ""],
        )
        .unwrap();
    }

    fn assistant_with_calls(calls: &[(&str, &str)]) -> Message {
        Message::assistant(
            None,
            calls
                .iter()
                .map(|(id, name)| ToolCall {
                    id: (*id).to_string(),
                    name: (*name).to_string(),
                    args: serde_json::json!({}),
                })
                .collect(),
        )
    }

    fn tool_result(id: &str) -> Message {
        Message::tool(id, "ok")
    }

    fn trailing_tool_rows(conn: &Connection, session_id: i64) -> Vec<Message> {
        let mut stmt = conn
            .prepare(
                "SELECT content_json FROM messages WHERE session_id = ?1 AND role = 'tool'
                 ORDER BY seq",
            )
            .unwrap();
        let rows = stmt
            .query_map([session_id], |row| row.get::<_, String>(0))
            .unwrap();
        rows.map(|r| serde_json::from_str::<Message>(&r.unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn dangling_pair_gets_synthetic_results_in_order() {
        let conn = open_mem();
        insert_session(&conn, 1);
        insert_msg(&conn, 1, 1, "user", &Message::user("chạy giúp hai lệnh"));
        insert_msg(
            &conn,
            1,
            2,
            "assistant",
            &assistant_with_calls(&[("a1", "run_shell"), ("a2", "run_shell")]),
        );

        // repair_interrupted_runs mở DB theo đường dẫn (không dùng được với
        // :memory: vì mỗi lần mở là DB khác) — test hành vi qua hàm nội bộ.
        let tx = conn.unchecked_transaction().unwrap();
        let n = repair_session_tail(&tx, 1).unwrap();
        tx.commit().unwrap();
        assert_eq!(n, 2);

        let tools = trailing_tool_rows(&conn, 1);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].tool_call_id.as_deref(), Some("a1"));
        assert_eq!(tools[1].tool_call_id.as_deref(), Some("a2"));
        assert!(tools[0].is_error && tools[1].is_error);
        assert_eq!(tools[0].text.as_deref(), Some(INTERRUPTED_NOTE));
    }

    #[test]
    fn partial_results_only_missing_appended() {
        let conn = open_mem();
        insert_session(&conn, 1);
        insert_msg(&conn, 1, 1, "user", &Message::user("go"));
        insert_msg(
            &conn,
            1,
            2,
            "assistant",
            &assistant_with_calls(&[("a1", "run_shell"), ("a2", "run_shell")]),
        );
        insert_msg(&conn, 1, 3, "tool", &tool_result("a1"));

        let tx = conn.unchecked_transaction().unwrap();
        let n = repair_session_tail(&tx, 1).unwrap();
        tx.commit().unwrap();
        assert_eq!(n, 1);

        let tools = trailing_tool_rows(&conn, 1);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[1].tool_call_id.as_deref(), Some("a2"));
        assert!(tools[1].is_error);
    }

    #[test]
    fn clean_tail_untouched() {
        let conn = open_mem();
        insert_session(&conn, 1);
        // Đuôi là user — run chưa kịp trả lời, không có cặp vỡ.
        insert_msg(
            &conn,
            1,
            1,
            "assistant",
            &assistant_with_calls(&[("a1", "ls")]),
        );
        insert_msg(&conn, 1, 2, "tool", &tool_result("a1"));
        insert_msg(&conn, 1, 3, "user", &Message::user("câu hỏi mới"));

        // Đuôi là assistant thuần văn bản — cặp nào cũng đã đóng.
        insert_session(&conn, 2);
        insert_msg(&conn, 2, 1, "user", &Message::user("hi"));
        insert_msg(
            &conn,
            2,
            2,
            "assistant",
            &Message::assistant(Some("chào bạn".into()), Vec::new()),
        );

        let tx = conn.unchecked_transaction().unwrap();
        assert_eq!(repair_session_tail(&tx, 1).unwrap(), 0);
        assert_eq!(repair_session_tail(&tx, 2).unwrap(), 0);
        tx.commit().unwrap();
    }

    #[test]
    fn sessions_are_independent() {
        let conn = open_mem();
        insert_session(&conn, 1);
        insert_msg(&conn, 1, 1, "user", &Message::user("go"));
        insert_msg(
            &conn,
            1,
            2,
            "assistant",
            &assistant_with_calls(&[("a1", "ls")]),
        );
        insert_session(&conn, 2);
        insert_msg(&conn, 2, 1, "user", &Message::user("sạch"));

        let tx = conn.unchecked_transaction().unwrap();
        let n = repair_session_tail(&tx, 1).unwrap() + repair_session_tail(&tx, 2).unwrap();
        tx.commit().unwrap();
        assert_eq!(n, 1);
        assert_eq!(trailing_tool_rows(&conn, 1).len(), 1);
        assert_eq!(trailing_tool_rows(&conn, 2).len(), 0);
    }
}
