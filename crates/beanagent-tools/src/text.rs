//! Tiện ích xử lý văn bản dùng chung (agents.md mục 22.9).
//!
//! Cắt chuỗi theo **số ký tự** (codepoint), không theo chỉ số byte — cắt theo byte tuỳ ý
//! sẽ panic với tiếng Việt có dấu hoặc emoji.

/// Cắt chuỗi ở tối đa `max_chars` ký tự, luôn dừng ở **ranh giới ký tự** UTF-8.
///
/// Trả về `None` nếu chuỗi không vượt quá `max_chars` (không cắt); ngược lại trả về
/// `(phần_giữ_lại, số_ký_tự_đã_cắt)`.
///
/// ```ignore
/// let s = "tiếng Việt 🦀";
/// let (kept, cut) = truncate_chars(s, 10).expect("dài hơn 10 ký tự");
/// assert_eq!(kept.chars().count(), 10);
/// assert_eq!(cut, s.chars().count() - 10);
/// ```
#[must_use]
pub fn truncate_chars(s: &str, max_chars: usize) -> Option<(&str, usize)> {
    for (seen, (idx, _)) in s.char_indices().enumerate() {
        if seen == max_chars {
            // `idx` là ranh giới ký tự (bắt đầu của một codepoint) nên slicing an toàn.
            let cut = s[idx..].chars().count();
            return Some((&s[..idx], cut));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::truncate_chars;

    #[test]
    fn no_truncation_when_short_enough() {
        assert!(truncate_chars("", 10).is_none());
        assert!(truncate_chars("xin chào", 8).is_none());
        assert!(truncate_chars("xin chào", 100).is_none());
    }

    #[test]
    fn cuts_at_char_boundary_with_vietnamese_and_emoji() {
        let s = "Tiếng Việt có dấu: ệảâ 🦀🦀🦀";
        let total = s.chars().count();
        let max = total - 3;
        let (kept, cut) = truncate_chars(s, max).expect("phải cắt");
        assert_eq!(kept.chars().count(), max);
        assert_eq!(cut, 3);
        // Phần giữ lại luôn là UTF-8 hợp lệ (không cắt giữa codepoint).
        assert_eq!(kept.chars().count() + cut, total);
    }

    #[test]
    fn emoji_are_four_bytes_but_count_as_one_char() {
        let s = "🦀".repeat(10); // 40 byte, 10 ký tự
        let (kept, cut) = truncate_chars(s.as_str(), 4).expect("phải cắt");
        assert_eq!(kept, "🦀🦀🦀🦀");
        assert_eq!(cut, 6);
    }

    #[test]
    fn zero_max_returns_empty() {
        let (kept, cut) = truncate_chars("abc", 0).expect("phải cắt");
        assert!(kept.is_empty());
        assert_eq!(cut, 3);
    }
}
