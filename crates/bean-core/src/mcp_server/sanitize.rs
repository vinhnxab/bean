//! Làm sạch tham số MCP client trước khi chạm tầng dưới (M25, mục 4).
//!
//! # Vấn đề
//!
//! Tham số do Cline/Cursor/… gửi tới là **input không tin cậy** (agents.md mục 15.4 và
//! nguyên tắc "query của model là dữ liệu không tin cậy" trong `docs/known-issues.md`).
//! Nguồn *là* một coding agent không làm nó trở thành dữ liệu đáng tin: prompt injection
//! trong repo có thể khiến agent đó gửi chuỗi độc hại, và `Plan.md` M25 yêu cầu rõ
//! *"không tin tưởng chỉ vì request tới từ một coding agent 'có vẻ đáng tin'"*.
//!
//! # Cách xử lý
//!
//! Lớp này **không** cố "hiểu" ý định client. Nó áp bốn giới hạn cấu trúc, đủ để một
//! tham số độc hại không đi nguyên văn xuống tầng dưới:
//!
//! 1. **Độ sâu** — JSON lồng sâu là đầu mối đệ quy/ngốn bộ nhớ ở tầng dưới.
//! 2. **Độ rộng** — object/array quá nhiều phần tử.
//! 3. **Độ dài chuỗi** — cắt tại ranh giới ký tự (mục 22.9: cắt byte tuỳ ý sẽ panic).
//! 4. **Ký tự điều khiển** — bỏ `\0` và ký tự C0/C1; chúng vô nghĩa với tham số hợp lệ
//!    nhưng lại gây rối log/audit và có thể lọt qua bộ lọc dựa trên ký tự đặc biệt.
//!
//! Câu truy vấn FTS/SQL vẫn được làm sạch **thêm một lần nữa** ở đúng chỗ dùng nó
//! (`sanitize_fts_query` trong `bean-memory`); lớp này không thay thế việc đó.

use serde_json::Value;

/// Độ sâu tối đa của JSON tham số.
pub const MAX_DEPTH: usize = 8;
/// Số phần tử tối đa của một object/array.
pub const MAX_ITEMS: usize = 64;
/// Số ký tự tối đa của một chuỗi.
pub const MAX_STRING_CHARS: usize = 2_000;

/// Làm sạch toàn bộ tham số client gửi tới.
///
/// Hàm **không** thất bại: giá trị vượt giới hạn bị cắt/bỏ, không phải lỗi — một
/// tham số hỏng phải cho client thấy kết quả "đã bị giới hạn", không phải chết session.
#[must_use]
pub fn sanitize_client_args(args: &Value) -> Value {
    sanitize_value(args, 0)
}

fn sanitize_value(value: &Value, depth: usize) -> Value {
    if depth >= MAX_DEPTH {
        return Value::String("[đã cắt do quá sâu]".to_string());
    }
    match value {
        Value::String(text) => Value::String(sanitize_string(text)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(MAX_ITEMS)
                .map(|item| sanitize_value(item, depth + 1))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len().min(MAX_ITEMS));
            for (key, item) in map.iter().take(MAX_ITEMS) {
                // Khoá cũng là dữ liệu không tin cậy (đi vào log/audit, và tool có thể
                // dùng làm tên tham số động).
                out.insert(sanitize_string(key), sanitize_value(item, depth + 1));
            }
            Value::Object(out)
        }
        // Số/boolean/null vô hại; chúng vẫn là JSON hợp lệ cho `schemars` validate.
        other => other.clone(),
    }
}

/// Cắt chuỗi và bỏ ký tự điều khiển.
fn sanitize_string(text: &str) -> String {
    let stripped: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    // `truncate_chars` cắt tại ranh giới UTF-8, trả `None` khi không cần cắt.
    bean_tools::truncate_chars(&stripped, MAX_STRING_CHARS)
        .map_or(stripped.clone(), |(kept, _)| kept.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn strips_injection_like_sql_and_fts_payloads_from_strings() {
        let args = serde_json::json!({
            "query": "'; DROP TABLE memories; --",
            "fts": "\" OR 1=1 OR \"",
        });
        let clean = sanitize_client_args(&args);
        // Chuỗi vẫn còn (không xoá dữ liệu người dùng) nhưng không chứa ký tự điều khiển.
        assert_eq!(clean["query"], "'; DROP TABLE memories; --");
        assert!(!clean["fts"].as_str().unwrap().contains('\0'));
    }

    #[test]
    fn removes_control_characters_and_truncates_at_char_boundary() {
        let with_control = "a\u{0}b\u{7}c";
        assert_eq!(sanitize_string(with_control), "abc");
        // Tiếng Việt + emoji: cắt không được làm hỏng UTF-8 (mục 22.9).
        let long = "á".repeat(MAX_STRING_CHARS + 50);
        let cut = sanitize_string(&long);
        assert!(cut.chars().count() <= MAX_STRING_CHARS);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
    }

    #[test]
    fn caps_depth_width_and_keeps_scalars() {
        let mut deep = serde_json::json!(1);
        for _ in 0..(MAX_DEPTH + 5) {
            deep = serde_json::json!({ "a": deep });
        }
        let rendered = sanitize_client_args(&deep).to_string();
        assert!(rendered.contains("quá sâu"), "{rendered}");

        let wide: Vec<Value> = (0..MAX_ITEMS + 20).map(Value::from).collect();
        let clean = sanitize_client_args(&Value::Array(wide));
        assert_eq!(clean.as_array().unwrap().len(), MAX_ITEMS);
    }
}
