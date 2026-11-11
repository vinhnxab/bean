//! Sinh **unified diff giới hạn độ dài** cho thẻ xác nhận (mục 12.1.4).
//!
//! Chỉ dùng để **hiển thị cho người dùng** khi duyệt `write_file`/`edit_file` —
//! không có ai parse lại output này, không dùng để áp dụng thay đổi. Vì vậy định
//! dạng ưu tiên *dễ đọc* hơn *chuẩn git tuyệt đối* (số dòng kiểu git vẫn được giữ
//! gần đúng cho trường thường gặp).
//!
//! Thuật toán: cắt common prefix/suffix, LCS (DP) trên đoạn giữa để tìm thay đổi
//! thật sự, gom thành hunk với 3 dòng context. Trần ô DP chặn file lớn: vượt thì
//! rơi về tóm tắt thay toàn bộ thay vì treo hoặc tốn GB bộ nhớ.

/// Số dòng context quanh mỗi cụm thay đổi trong một hunk.
const CONTEXT_LINES: usize = 3;
/// Trần ô DP cho LCS trên đoạn giữa (sau khi cắt prefix/suffix).
const MAX_DP_CELLS: usize = 1_000_000;
/// Trần dòng hiển thị mặc định cho thẻ xác nhận.
pub const DEFAULT_MAX_LINES: usize = 50;

/// Unified diff `old` → `new`, giới hạn `max_lines` dòng nội dung (không tính
/// dòng header `---`/`+++` và các dòng hunk `@@`). Hai file giống hệt ⇒ trả
/// "(nội dung không đổi)".
#[must_use]
pub fn unified_diff_bounded(old: &str, new: &str, path: &str, max_lines: usize) -> String {
    if old == new {
        return "(nội dung không đổi)\n".to_string();
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    if old_lines == new_lines {
        return "(chỉ khác ký tự xuống dòng ở cuối file)\n".to_string();
    }

    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];

    // Dãy op đầy đủ: prefix + đoạn-giữa + suffix. Prefix/suffix giữ nguyên dưới dạng
    // `Equal` (tham chiếu, không tốn DP) để hunks có **context thật** hai phía — chỉ
    // LCS chạy trên đoạn giữa, đúng trần ô đã đặt.
    // `extra_lines`: số dòng nội dung bị ẩn ngoài `body` (chỉ fallback dùng, vì nó
    // tự cắt trước để không dựng hàng chục nghìn `String` chỉ để vứt đi).
    let (body, extra_lines): (Vec<String>, usize) = if old_mid.len() * new_mid.len() > MAX_DP_CELLS
    {
        // Fallback: đoạn giữa quá lớn để LCS — hiển thị tóm tắt + đầu bản mới.
        let shown = max_lines.saturating_sub(1);
        let mut body = vec![format!(
            "@@ thay toàn bộ đoạn giữa ({} dòng → {} dòng, quá lớn để diff chi tiết) @@",
            old_mid.len(),
            new_mid.len()
        )];
        body.extend(new_mid.iter().take(shown).map(|line| format!("+{line}")));
        (body, new_mid.len().saturating_sub(shown))
    } else {
        let mut ops: Vec<Op<'_>> = old_lines[..prefix]
            .iter()
            .map(|line| Op::Equal(line))
            .collect();
        ops.extend(lcs_ops(old_mid, new_mid));
        ops.extend(
            old_lines[old_lines.len() - suffix..]
                .iter()
                .map(|line| Op::Equal(line)),
        );
        (render_hunks(&ops, 1, 1), 0)
    };
    if body.is_empty() {
        return "(chỉ khác ký tự xuống dòng ở cuối file)\n".to_string();
    }

    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    let mut emitted = 0usize;
    let mut skipped = 0usize;
    for line in &body {
        // Header hunk `@@` là metadata, không tính vào trần nội dung.
        if line.starts_with("@@") {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if emitted >= max_lines {
            skipped += 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
        emitted += 1;
    }
    if skipped > 0 {
        out.push_str(&format!("… (còn {skipped} dòng diff nữa)\n"));
    }
    if extra_lines > 0 {
        out.push_str(&format!("… (còn {extra_lines} dòng diff nữa)\n"));
    }
    out
}

enum Op<'a> {
    Equal(&'a str),
    Del(&'a str),
    Ins(&'a str),
}

/// Dãy op từ LCS của `old`/`new` (đã cắt prefix/suffix, DP nhỏ hơn trần).
fn lcs_ops<'a>(old: &'a [&'a str], new: &'a [&'a str]) -> Vec<Op<'a>> {
    let n = old.len();
    let m = new.len();
    let width = m + 1;
    let mut dp = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * width + j] = if old[i] == new[j] {
                dp[(i + 1) * width + (j + 1)].saturating_add(1)
            } else {
                dp[(i + 1) * width + j].max(dp[i * width + (j + 1)])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            ops.push(Op::Equal(old[i]));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * width + j] >= dp[i * width + (j + 1)] {
            ops.push(Op::Del(old[i]));
            i += 1;
        } else {
            ops.push(Op::Ins(new[j]));
            j += 1;
        }
    }
    for line in old_lines_tail(old, i) {
        ops.push(Op::Del(line));
    }
    for line in new_lines_tail(new, j) {
        ops.push(Op::Ins(line));
    }
    ops
}

fn old_lines_tail<'a>(old: &'a [&'a str], from: usize) -> &'a [&'a str] {
    &old[from.min(old.len())..]
}

fn new_lines_tail<'a>(new: &'a [&'a str], from: usize) -> &'a [&'a str] {
    &new[from.min(new.len())..]
}

/// Gom dãy op thành hunk chuẩn unified (3 dòng context, gom cụm cách nhau
/// ≤ `2 * CONTEXT_LINES` dòng), đánh số dòng 1-based tính từ `first_line_no`.
fn render_hunks(ops: &[Op<'_>], first_old_no: usize, first_new_no: usize) -> Vec<String> {
    // Số dòng old/new đã tiêu thụ trước mỗi op.
    let mut old_no = vec![first_old_no; ops.len() + 1];
    let mut new_no = vec![first_new_no; ops.len() + 1];
    for (index, op) in ops.iter().enumerate() {
        old_no[index + 1] = old_no[index] + usize::from(!matches!(op, Op::Ins(_)));
        new_no[index + 1] = new_no[index] + usize::from(!matches!(op, Op::Del(_)));
    }
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, Op::Equal(_)))
        .map(|(index, _)| index)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut group_start = 0usize;
    while group_start < changed.len() {
        let mut group_end = group_start + 1;
        while group_end < changed.len()
            && changed[group_end] - changed[group_end - 1] <= 2 * CONTEXT_LINES
        {
            group_end += 1;
        }
        let first_change = changed[group_start];
        let last_change = changed[group_end - 1];
        let start = first_change.saturating_sub(CONTEXT_LINES);
        let end = (last_change + CONTEXT_LINES + 1).min(ops.len());

        let old_start = old_no[start];
        let new_start = new_no[start];
        let old_count = ops[start..end]
            .iter()
            .filter(|op| !matches!(op, Op::Ins(_)))
            .count();
        let new_count = ops[start..end]
            .iter()
            .filter(|op| !matches!(op, Op::Del(_)))
            .count();
        out.push(format!(
            "@@ -{},{} +{},{} @@",
            if old_count == 0 {
                old_start.saturating_sub(1)
            } else {
                old_start
            },
            old_count,
            if new_count == 0 {
                new_start.saturating_sub(1)
            } else {
                new_start
            },
            new_count
        ));
        for op in &ops[start..end] {
            match op {
                Op::Equal(line) => out.push(format!(" {line}")),
                Op::Del(line) => out.push(format!("-{line}")),
                Op::Ins(line) => out.push(format!("+{line}")),
            }
        }
        group_start = group_end;
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{DEFAULT_MAX_LINES, unified_diff_bounded};

    #[test]
    fn identical_content_reports_no_change() {
        assert_eq!(
            unified_diff_bounded("a\nb\n", "a\nb\n", "f.txt", DEFAULT_MAX_LINES),
            "(nội dung không đổi)\n"
        );
    }

    #[test]
    fn only_trailing_newline_differs_reports_a_note() {
        assert_eq!(
            unified_diff_bounded("a\nb", "a\nb\n", "f.txt", DEFAULT_MAX_LINES),
            "(chỉ khác ký tự xuống dòng ở cuối file)\n"
        );
    }

    #[test]
    fn new_file_shows_all_lines_added() {
        let out = unified_diff_bounded("", "dòng 1\ndòng 2\n", "moi.md", DEFAULT_MAX_LINES);
        assert!(out.starts_with("--- a/moi.md\n+++ b/moi.md\n"));
        assert!(out.contains("@@ -0,0 +1,2 @@"), "{out}");
        assert!(out.contains("+dòng 1") && out.contains("+dòng 2"), "{out}");
    }

    #[test]
    fn middle_change_keeps_context_and_line_numbers() {
        let old = (1..=10).map(|i| format!("dòng {i}")).collect::<Vec<_>>();
        let old = format!("{}\n", old.join("\n"));
        let mut new_lines: Vec<String> = (1..=10).map(|i| format!("dòng {i}")).collect();
        new_lines[4] = "dòng 5 đã sửa".into();
        let new = format!("{}\n", new_lines.join("\n"));
        let out = unified_diff_bounded(&old, &new, "f.txt", DEFAULT_MAX_LINES);
        // Hunk gồm dòng 2..=10 (context 3 quanh dòng 5), đủ context hai phía.
        assert!(out.contains("@@ -2,7 +2,7 @@"), "{out}");
        assert!(out.contains("-dòng 5\n+dòng 5 đã sửa\n"), "{out}");
        assert!(
            out.contains(" dòng 2\n") && out.contains(" dòng 8\n"),
            "{out}"
        );
        assert!(!out.contains("dòng 1\n"), "dòng 1 ngoài context: {out}");
    }

    #[test]
    fn two_separate_changes_make_two_hunks() {
        let old: String = (1..=20).map(|i| format!("dòng {i}\n")).collect();
        let new: String = (1..=20)
            .map(|i| {
                if i == 2 {
                    "HAI".to_string()
                } else if i == 18 {
                    "MƯỜI TÁM".to_string()
                } else {
                    format!("dòng {i}")
                }
            })
            .map(|line| format!("{line}\n"))
            .collect();
        let out = unified_diff_bounded(&old, &new, "f.txt", DEFAULT_MAX_LINES);
        assert_eq!(out.matches("@@").count(), 4, "2 hunk × header: {out}");
        assert!(out.contains("-dòng 2\n+HAI\n"), "{out}");
        assert!(out.contains("-dòng 18\n+MƯỜI TÁM\n"), "{out}");
    }

    #[test]
    fn truncates_at_max_lines_with_note() {
        let new = (1..=200).map(|i| format!("dòng {i}\n")).collect::<String>();
        let out = unified_diff_bounded("", &new, "f.txt", 10);
        assert!(out.contains("… (còn"), "{out}");
        assert_eq!(out.lines().count(), 3 + 10 + 1, "{out}"); // 2 header + 1 header hunk + 10 + 1 note
    }

    #[test]
    fn huge_middle_falls_back_to_summary_without_hanging() {
        // 2000 dòng khác hoàn toàn nhau (sau prefix/suffix trống) ⇒ vượt trần DP.
        let old: String = (0..2000).map(|i| format!("cũ {i}\n")).collect();
        let new: String = (0..2000).map(|i| format!("mới {i}\n")).collect();
        let out = unified_diff_bounded(&old, &new, "f.txt", 30);
        assert!(out.contains("quá lớn để diff chi tiết"), "{out}");
        assert!(out.contains("+mới 0"), "{out}");
        assert!(out.contains("… (còn"), "{out}");
    }

    #[test]
    fn vietnamese_and_emoji_do_not_panic_and_keep_characters() {
        let old = "Xin chào 🧸 thế giới\ndòng thường\n";
        let new = "Xin chào 🧸 thế giới ệ ư ơ\nđổi ở đây 🎏\ndòng thường\n";
        let out = unified_diff_bounded(old, new, "tiếng-việt.md", DEFAULT_MAX_LINES);
        assert!(out.contains("+đổi ở đây 🎏"), "{out}");
        assert!(out.contains("-Xin chào 🧸 thế giới"), "{out}");
        assert!(out.contains("+Xin chào 🧸 thế giới ệ ư ơ"), "{out}");
    }

    #[test]
    fn edit_replacement_inside_line_shows_hunk() {
        let old = "fn main() {\n    println!(\"xin chào\");\n}\n";
        let new = "fn main() {\n    println!(\"tạm biệt\");\n}\n";
        let out = unified_diff_bounded(old, new, "src/main.rs", DEFAULT_MAX_LINES);
        assert!(out.contains("-    println!(\"xin chào\");"), "{out}");
        assert!(out.contains("+    println!(\"tạm biệt\");"), "{out}");
        assert!(out.contains(" fn main() {"), "{out}");
    }
}
