//! Ranh giới an toàn khi cắt lịch sử message (agents.md mục 8.3, 22.1).
//!
//! Bất biến quan trọng nhất của cả tầng bộ nhớ: **không bao giờ** giữ một `tool` result
//! mà assistant message gọi ra nó đã bị loại bỏ. Vi phạm bất biến này khiến provider trả
//! 400 và cả phiên hội thoại hỏng.

use beanagent_types::{Message, Role};

/// Index bắt đầu của đoạn **giữ lại** khi cần bỏ bớt lịch sử, đảm bảo không có `tool`
/// result mồ côi.
///
/// Xuất phát từ `len - desired_keep` rồi tiến về phía sau cho tới khi gặp message không
/// phải `Role::Tool` (một `tool` result tại điểm cắt nghĩa là assistant gọi ra nó đã bị
/// loại — phải bỏ luôn result đó).
///
/// Trả về `messages.len()` khi phải bỏ toàn bộ (đoạn giữ lại rỗng).
#[must_use]
pub fn find_safe_start(messages: &[Message], desired_keep: usize) -> usize {
    let len = messages.len();
    let mut start = len.saturating_sub(desired_keep);
    while start < len && messages[start].role == Role::Tool {
        start += 1;
    }
    start.min(len)
}

/// Điểm cắt dùng cho **compaction**: ranh giới an toàn theo nghĩa mục 8.3 — ngay trước
/// một message `User` — và vẫn thoả [`check_no_orphan_result`].
///
/// Trả `None` khi không tồn tại ranh giới như vậy (khi đó nên **giữ nguyên** lịch sử
/// thay vì cắt liều).
#[must_use]
pub fn find_compaction_start(messages: &[Message], desired_keep: usize) -> Option<usize> {
    let len = messages.len();
    if len == 0 {
        return None;
    }
    let from = len.saturating_sub(desired_keep);
    (from..len).find(|&index| {
        messages[index].role == Role::User && check_no_orphan_result(messages, index)
    })
}

/// Điểm cắt **lùi về trước** cho tới khi an toàn: dùng khi cần *giữ* mức tối thiểu
/// `desired_keep` message (ngân sách context, mục 8.2) — nếu message tại điểm cắt là
/// `tool` result thì lùi thêm cho tới khi gặp assistant gọi ra nó.
///
/// Khác [`find_safe_start`] (tiến về sau, bỏ thêm message): hàm này **giữ thêm** để không
/// bao giờ tách cặp, và luôn giữ ít nhất message mới nhất khi `messages` không rỗng.
#[must_use]
pub fn extend_start_backwards(messages: &[Message], desired_keep: usize) -> usize {
    let len = messages.len();
    if len == 0 {
        return 0;
    }
    let mut start = len.saturating_sub(desired_keep).min(len - 1);
    while start > 0 && messages[start].role == Role::Tool {
        start -= 1;
    }
    start
}

/// Kiểm tra bất biến: đoạn `messages[start..]` không chứa `tool` result mồ côi.
#[must_use]
pub fn check_no_orphan_result(messages: &[Message], start: usize) -> bool {
    let Some(retained) = messages.get(start..) else {
        return false;
    };
    let mut expected: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for msg in retained {
        if msg.role == Role::Assistant {
            for call in &msg.tool_calls {
                expected.insert(call.id.as_str());
            }
        }
    }
    retained.iter().all(|msg| {
        msg.role != Role::Tool
            || msg
                .tool_call_id
                .as_deref()
                .is_none_or(|id| expected.contains(id))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{
        check_no_orphan_result, extend_start_backwards, find_compaction_start, find_safe_start,
    };
    use beanagent_types::{Message, Role, ToolCall};
    use proptest::prelude::*;

    fn assistant_calls(ids: &[&str]) -> Message {
        Message::assistant(
            None,
            ids.iter()
                .map(|id| ToolCall::new(*id, "probe", serde_json::json!({ "n": 1 })))
                .collect(),
        )
    }

    fn assistant_text(text: &str) -> Message {
        Message::assistant(Some(text.to_string()), Vec::new())
    }

    /// Dựng lịch sử **well-formed**: mọi assistant có `tool_calls` đều được theo sau bởi
    /// đúng các `tool` result của nó (`count = 0` ⇒ assistant trả lời thẳng).
    fn build_history(steps: &[usize]) -> Vec<Message> {
        let mut out = vec![Message::user("bắt đầu")];
        for (step, count) in steps.iter().enumerate() {
            if *count == 0 {
                out.push(assistant_text(&format!("trả lời {step}")));
                continue;
            }
            let ids: Vec<String> = (0..*count).map(|n| format!("c{step}-{n}")).collect();
            let calls: Vec<ToolCall> = ids
                .iter()
                .map(|id| ToolCall::new(id.clone(), "probe", serde_json::json!({ "n": 1 })))
                .collect();
            out.push(Message::assistant(None, calls));
            for id in &ids {
                out.push(Message::tool(id.clone(), "ok"));
            }
        }
        out
    }

    #[test]
    fn safe_start_keeps_pairs_together() {
        // [user, assistant(c1), tool(c1)] — cắt 1 message sẽ rơi vào tool ⇒ phải bỏ luôn.
        let history = vec![
            Message::user("a"),
            assistant_calls(&["c1"]),
            Message::tool("c1", "ok"),
        ];
        assert_eq!(find_safe_start(&history, 1), 3);
        assert_eq!(find_safe_start(&history, 2), 1);
        assert!(check_no_orphan_result(&history, 1));
        // Rơi vào assistant call là an toàn (result của nó cũng được giữ).
        assert_eq!(find_safe_start(&history, 3), 0);
    }

    #[test]
    fn safe_start_on_empty_history() {
        assert_eq!(find_safe_start(&[], 5), 0);
        assert!(find_compaction_start(&[], 5).is_none());
    }

    #[test]
    fn compaction_start_is_user_boundary() {
        let history = vec![
            Message::user("một"),
            assistant_calls(&["c1"]),
            Message::tool("c1", "ok"),
            assistant_text("xong"),
            Message::user("hai"),
            assistant_calls(&["c2", "c3"]),
            Message::tool("c2", "ok"),
            Message::tool("c3", "lỗi"),
            Message::user("ba"),
        ];
        // Giữ 4 message cuối ⇒ điểm cắt đầu tiên là `user("ba")`.
        assert_eq!(find_compaction_start(&history, 4), Some(8));
        // Giữ 2 ⇒ `user("ba")` (không có user nào gần hơn).
        assert_eq!(find_compaction_start(&history, 2), Some(8));
        // Giữ tất cả ⇒ cắt ở đầu (không có gì để nén).
        assert_eq!(find_compaction_start(&history, history.len()), Some(0));
        for keep in 0..=history.len() {
            if let Some(start) = find_compaction_start(&history, keep) {
                assert_eq!(history[start].role, Role::User);
                assert!(check_no_orphan_result(&history, start));
                assert!(start + keep >= history.len());
            }
        }
    }

    #[test]
    fn compaction_start_none_without_user_message() {
        let history = vec![assistant_calls(&["c1"]), Message::tool("c1", "ok")];
        assert_eq!(find_compaction_start(&history, 1), None);
    }

    #[test]
    fn extend_backwards_keeps_pairs_and_newest_message() {
        let history = vec![
            Message::user("a"),
            assistant_calls(&["c1"]),
            Message::tool("c1", "ok"),
            assistant_text("xong"),
        ];
        // Giữ 1 message cuối là assistant ⇒ an toàn, không phải lùi.
        assert_eq!(extend_start_backwards(&history, 1), 3);
        // Điểm cắt rơi vào tool result ⇒ phải lùi về assistant gọi ra nó.
        assert_eq!(extend_start_backwards(&history, 2), 1);
        assert_eq!(extend_start_backwards(&history, 3), 1);
        assert_eq!(extend_start_backwards(&history, 99), 0);
        // Rỗng ⇒ 0.
        assert_eq!(extend_start_backwards(&[], 5), 0);
        for keep in 0..=history.len() {
            let start = extend_start_backwards(&history, keep);
            assert!(start < history.len() || history.is_empty());
            assert!(history[start].role != Role::Tool);
            assert!(check_no_orphan_result(&history, start));
        }
    }

    proptest! {
        /// Bất biến chính (mục 8.3): cắt lịch sử không bao giờ tạo `tool` result mồ côi.
        #[test]
        fn safe_start_never_orphans_result(
            steps in proptest::collection::vec(0usize..4, 0..12),
            keep in 0usize..30,
        ) {
            let history = build_history(&steps);
            let start = find_safe_start(&history, keep);
            prop_assert!(start <= history.len());
            prop_assert!(
                check_no_orphan_result(&history, start),
                "cắt tại {} / {} message",
                start,
                history.len()
            );
        }

        /// Compaction cũng vậy, và điểm cắt phải nằm trên một message `User`.
        #[test]
        fn compaction_start_is_safe(
            steps in proptest::collection::vec(0usize..4, 0..12),
            keep in 0usize..30,
        ) {
            let history = build_history(&steps);
            if let Some(start) = find_compaction_start(&history, keep) {
                prop_assert_eq!(history[start].role, Role::User);
                prop_assert!(check_no_orphan_result(&history, start));
                prop_assert!(start + keep >= history.len());
            }
        }
    }
}
