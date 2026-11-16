//! Làm sạch query FTS5 trước khi đưa vào `MATCH` — ranh giới giữa đầu vào không tin cậy
//! (query của model) và cú pháp FTS (agents.md mục 8.4, K9).

/// Làm sạch query của người dùng/model trước khi đưa vào `MATCH` (agents.md mục 8.4).
///
/// Coi mọi ký hiệu (dấu câu, toán tử FTS, emoji) là **ranh giới từ** và tách query thành
/// các dãy ký tự chữ–số liền nhau (kể cả dấu tiếng Việt; `_` giữ trong dãy), bọc mỗi dãy
/// trong `\"`; nhiều dãy nối bằng dấu cách = AND ngầm. Nhờ vậy cú pháp FTS không thể bị
/// phá, và từ có ký hiệu không còn bị nối liền thành từ không tồn tại (K9):
/// `rust-lang` → `\"rust\" \"lang\"` (không phải `\"rustlang\"`), `C++` → `\"C\"`.
/// Trả `None` khi không còn dãy nào (`\"+++\"`, `\"\"`) — khi đó tìm kiếm trả về rỗng thay
/// vì lỗi cú pháp.
pub(super) fn sanitize_fts_query(query: &str) -> Option<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(format!("\"{current}\""));
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(format!("\"{current}\""));
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::sanitize_fts_query;

    /// (K9) Mọi ký hiệu là ranh giới từ — không nối các ký tự còn lại thành một từ.
    #[test]
    fn ky_hieu_la_ranh_gioi_tu_khong_noi_lien() {
        let cases: [(&str, Option<&str>); 12] = [
            ("rust-lang", Some("\"rust\" \"lang\"")),
            ("C++", Some("\"C\"")),
            ("bean-2.0", Some("\"bean\" \"2\" \"0\"")),
            ("foo_bar", Some("\"foo_bar\"")),
            ("--rust", Some("\"rust\"")),
            ("hồ sơ", Some("\"hồ\" \"sơ\"")),
            ("café:test*", Some("\"café\" \"test\"")),
            ("😀 emoji", Some("\"emoji\"")),
            // Ký tự điều khiển FTS (`"`, `=`, `-`, `OR` giữ nguyên thành từ thường) —
            // vẫn không thể phá cú pháp vì mọi dãy đều được bọc `\"`.
            (
                "dự án\" OR 1=1 --",
                Some("\"dự\" \"án\" \"OR\" \"1\" \"1\""),
            ),
            ("", None),
            ("   ", None),
            ("+++", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                sanitize_fts_query(input).as_deref(),
                expected,
                "input: {input:?}"
            );
        }
    }
}
