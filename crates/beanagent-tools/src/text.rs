//! Tiện ích xử lý văn bản dùng chung (agents.md mục 22.9).
//!
//! Cắt chuỗi theo **số ký tự** (codepoint), không theo chỉ số byte — cắt theo byte tuỳ ý
//! sẽ panic với tiếng Việt có dấu hoặc emoji.

use crate::error::ToolError;

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

/// Biên dịch regex cho tool `grep` — lỗi cú pháp trở thành [`ToolError::InvalidArgs`]
/// (model có thể tự sửa tham số).
///
/// # Errors
/// [`ToolError::InvalidArgs`] khi pattern không phải regex hợp lệ.
pub fn compile_regex(pattern: &str) -> Result<regex::Regex, ToolError> {
    regex::Regex::new(pattern)
        .map_err(|e| ToolError::InvalidArgs(format!("regex không hợp lệ `{pattern}`: {e}")))
}

/// Bỏ mọi ký tự điều khiển C0/C1 và chuỗi escape `ESC`/`OSC` khỏi văn bản trước khi in
/// ra terminal (S2, `docs/security-review.md` mục 3.2).
///
/// # Vì sao cần
///
/// Output của tool **không tin cậy** (`run_shell`, `read_file`, `web_fetch`, MCP…) là dữ
/// liệu kẻ tấn công kiểm soát được. Nếu in thẳng ra terminal, chuỗi ANSI/OSC có thể:
///
/// * vẽ lại màn hình, xoá dòng, **che giấu prompt xác nhận** để người dùng bấm nhầm;
/// * dùng **OSC 52** để cài sẵn nội dung clipboard, khiến người dùng dán nhầm lệnh khác.
///
/// # Nguyên tắc
///
/// Chỉ loại **ký tự điều khiển** và **nội dung chuỗi escape**, không sửa chữ thường. Giữ
/// nguyên `\n`/`\t` để output nhiều dòng vẫn đọc được.
///
/// Khi gặp `ESC` ta **nuốt trọn chuỗi escape** do nó mở đầu, không chỉ bỏ ký tự mở đầu.
/// Nếu chỉ bỏ `ESC` thì phần tham số còn lại (`]52;c;Y3VjdG9y`) lọt ra terminal dưới
/// dạng chữ thường vô nghĩa — an toàn hơn là bỏ luôn, và cũng tránh hiển thị rác cho
/// người dùng. Chuỗi được nhận diện:
///
/// | Mở đầu | Phần thân | Kết thúc |
/// |---|---|---|
/// | `ESC ]`, `ESC P`, `ESC X`, `ESC ^`, `ESC _` | OSC/DCS/SOS/PM/APC | `BEL` hoặc `ST` (`ESC \`) |
/// | `ESC [` | CSI | byte cuối trong khoảng `0x40`–`0x7E` |
/// | `ESC` + 1 ký tự khác | hai ký tự đơn (`ESC c`, `ESC 7`…) | — |
///
/// Cùng cơ chế áp dụng cho C1 8-bit (`U+009B`/`U+009D`…). Chạy trên `char` nên không bao
/// giờ cắt giữa codepoint (mục 22.9).
#[must_use]
pub fn strip_terminal_escapes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        // Các nhánh dưới đều trả vị trí **sau** chuỗi escape ⇒ phải `continue` luôn. Nếu
        // rơi xuống `i += 1` ở cuối vòng thì sẽ nhảy qua một ký tự thật của output.
        if ch == '\u{1B}' {
            i = consume_escape(&chars, i);
            continue;
        }
        match ch {
            // Giữ ký tự trắng có ý nghĩa hiển thị; bỏ hết phần còn lại của C0.
            '\n' | '\t' => out.push(ch),
            // DEL.
            '\u{7F}' => {}
            // C1: CSI 8-bit (U+009B) và OSC/DCS 8-bit (U+009D/90/98/9E/9F) cũng mở đầu
            // chuỗi escape, nên phải nuốt cả phần thân chứ không chỉ bỏ ký tự đó.
            '\u{9B}' => {
                i = consume_csi(&chars, i + 1);
                continue;
            }
            '\u{9D}' | '\u{90}' | '\u{98}' | '\u{9E}' | '\u{9F}' => {
                i = consume_string_seq(&chars, i + 1);
                continue;
            }
            c if c.is_control() => {}
            _ => out.push(ch),
        }
        i += 1;
    }
    out
}

/// `ESC` đã bị tiêu thụ tại `chars[i]`; trả vị trí ngay **sau** chuỗi escape.
fn consume_escape(chars: &[char], i: usize) -> usize {
    let Some(&next) = chars.get(i + 1) else {
        return i + 1; // `ESC` lơ lửng ở cuối chuỗi.
    };
    match next {
        '[' => consume_csi(chars, i + 2),
        ']' | 'P' | 'X' | '^' | '_' => consume_string_seq(chars, i + 2),
        // `ESC` + một ký tự bất kỳ (kể cả ký tự điều khiển) là chuỗi hai ký tự.
        _ => i + 2,
    }
}

/// Nuốt phần thân CSI tới byte kết thúc (byte cuối nằm trong `0x40`–`0x7E`).
fn consume_csi(chars: &[char], mut i: usize) -> usize {
    while let Some(&c) = chars.get(i) {
        i += 1;
        if ('\u{40}'..='\u{7E}').contains(&c) {
            break;
        }
    }
    i
}

/// Nuốt phần thân OSC/DCS tới `BEL`, `ST` 8-bit (`U+009C`) hoặc `ST` 7-bit (`ESC \`).
fn consume_string_seq(chars: &[char], mut i: usize) -> usize {
    while let Some(&c) = chars.get(i) {
        if c == '\u{7}' || c == '\u{9C}' {
            return i + 1;
        }
        if c == '\u{1B}' {
            // `ST` = `ESC \`; nếu không phải thì coi như kết thúc luôn để tránh nuốt
            // hết phần còn lại của output.
            return if chars.get(i + 1) == Some(&'\\') {
                i + 2
            } else {
                i + 1
            };
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{strip_terminal_escapes, truncate_chars};

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

    /// S2: chuỗi escape phải bị bỏ hẳn, không được lọt xuống terminal.
    #[test]
    fn escape_sequences_are_removed() {
        // OSC 52 — cài nội dung clipboard (đúng kịch bản tấn công trong security-review),
        // theo sau là CSI xoá màn hình để che prompt xác nhận.
        let attack = "xin chào\u{1B}]52;c;Y3VjdG9y\x07\r\n\u{1B}[2Jkết thúc\u{7F}";
        let clean = strip_terminal_escapes(attack);
        assert_eq!(clean, "xin chào\nkết thúc");
        assert!(
            !clean.chars().any(|c| c.is_control() && c != '\n'),
            "không còn ký tự điều khiển nào ngoài xuống dòng: {clean:?}"
        );
        // Phần thân chuỗi escape cũng phải bị nuốt, không lọt ra dưới dạng chữ thường.
        assert!(
            !clean.contains("Y3VjdG9y"),
            "payload OSC 52 phải bị loại: {clean:?}"
        );
        assert!(
            !clean.contains("[2J"),
            "payload CSI phải bị loại: {clean:?}"
        );
    }

    /// S2: output bình thường không có escape thì **giữ nguyên y hệt** — bộ lọc không được
    /// cắt hay đổi nội dung (tiếng Việt, emoji, backslash, dấu ngoặc).
    #[test]
    fn plain_output_is_untouched() {
        for plain in [
            "",
            "Đã ghi xong file.",
            "Tiếng Việt có dấu: tiếng 🦀 emoji",
            "C:\\Users\\test\\file.txt",
            "dòng 1\ndòng 2\n\tthụt lề",
            "kết quả: 3/3 PASS, thời gian 1.2s",
            // Dấu `[` `]` hiện **không** đi kèm ESC vẫn phải giữ nguyên.
            "mảng [0, 1, 2] và khối {a: b}",
            "kết thúc bằng dấu gạch chân _ và backtick `",
        ] {
            assert_eq!(strip_terminal_escapes(plain), plain);
        }
    }

    /// S2: 8-bit C1 (CSI/OSC dạng U+009B/U+009D) cũng phải bị nuốt trọn chuỗi.
    #[test]
    fn c1_control_characters_are_removed() {
        assert_eq!(
            strip_terminal_escapes("a\u{9B}2Jb\u{9D}0;c;Zm9v\u{9C}c\u{85}d"),
            "abcd"
        );
    }

    /// S2: chuỗi escape không bị kết thúc không được nuốt hết phần output còn lại.
    #[test]
    fn unterminated_escape_does_not_swallow_following_output() {
        // OSC mở nhưng không có BEL/ST → chỉ bỏ phần tới hết chuỗi.
        assert_eq!(strip_terminal_escapes("a\u{1B}]52;c;dang"), "a");
        // `ESC` lơ lửng ở cuối.
        assert_eq!(strip_terminal_escapes("a\u{1B}"), "a");
        // OSC kết thúc bằng ST (`ESC \`) thì phần sau phải còn nguyên.
        assert_eq!(strip_terminal_escapes("a\u{1B}]0;title\u{1B}\\b"), "ab");
    }
}
