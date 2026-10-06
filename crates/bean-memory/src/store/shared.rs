//! Hàm dùng chung cho **cả hai** bản cài đặt ([`MemoryStore`](super::MemoryStore) và
//! [`SqliteStore`](super::SqliteStore)).
//!
//! # Vì sao tách khỏi [`super`]
//!
//! Đây là mối duy nhất hai bản cài đặt cùng dùng: `title_from` (tên hiển thị phiên),
//! `slice_start`/`sql_limit` (cùng một nghĩa "lấy N message cuối"), và các phép đo
//! token cho compaction. Chúng **phải** giống nhau tuyệt đối — nếu hai bản lệch nhau
//! thì một test chỉ chạy trên `MemoryStore` sẽ không bắt được hành vi sai của SQLite.
//!
//! Giữ chúng ở một chỗ khiến bất biến đó **hiện diện trong cấu trúc thư mục**: muốn
//! sửa cách cắt lịch sử thì có đúng một file phải đọc, thay vì phải nhớ "à, còn một
//! bản nữa ở trong file SQLite".

use bean_types::{Message, Role};

use super::CHARS_PER_TOKEN;

// ---------------------------------------------------------------------------
// Helper dùng chung cho cả hai bản cài đặt
// ---------------------------------------------------------------------------

/// Tiêu đề hiển thị trong UI, lấy từ **dòng đầu không rỗng** của tin đầu tiên do người
/// dùng gửi (agents.md mục 8.1). Trả `None` nếu message không phải của người dùng.
pub(crate) fn title_from(msg: &Message) -> Option<String> {
    if msg.role != Role::User {
        return None;
    }
    let text = msg.text.as_deref().unwrap_or_default();
    let first_line = text.lines().find(|line| !line.trim().is_empty())?;
    Some(truncate_chars(first_line.trim(), 60))
}

/// Cắt chuỗi tại **ranh giới ký tự** (agents.md mục 22.9) — không bao giờ panic với
/// tiếng Việt hay emoji.
pub(crate) fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// `before_seq` (1-based) ⇒ số phần tử cần giữ ở đầu danh sách.
pub(crate) fn seq_to_index(before_seq: Option<u64>, len: usize) -> usize {
    match before_seq {
        Some(seq) => (seq.saturating_sub(1) as usize).min(len),
        None => len,
    }
}

/// Vị trí bắt đầu của cửa sổ `limit` phần tử cuối (`limit = 0` ⇒ từ đầu).
pub(crate) fn slice_start(upper: usize, limit: usize) -> usize {
    if limit == 0 {
        0
    } else {
        upper.saturating_sub(limit)
    }
}

/// `LIMIT` cho SQLite: `0` trong API nghĩa là "không giới hạn" ⇒ `-1`.
pub(crate) fn sql_limit(limit: usize) -> i64 {
    if limit == 0 { -1 } else { limit as i64 }
}

/// Ước lượng token theo `chars / 4` khi provider không có API đếm (agents.md mục 8.2).
pub(crate) fn estimate_tokens(messages: &[Message]) -> u64 {
    let chars: u64 = messages
        .iter()
        .map(|msg| msg.text_for_search().chars().count() as u64)
        .sum();
    chars / CHARS_PER_TOKEN
}

/// Chuyển lịch sử thành transcript cho prompt tóm tắt (kèm hành động tool).
pub(crate) fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for msg in messages {
        out.push_str(msg.role.as_str());
        out.push_str(": ");
        if let Some(text) = &msg.text {
            out.push_str(text);
        }
        for call in &msg.tool_calls {
            out.push_str(&format!("\n  [gọi {}] {}", call.name, call.args));
        }
        if msg.is_error {
            out.push_str("\n  (lỗi)");
        }
        out.push('\n');
    }
    out
}

/// Mốc thời gian hiện tại ở định dạng RFC3339 UTC (micro giây).
///
/// Ở đây chứ không phải trong `sqlite/` vì **cả hai** bản cài đặt đều dùng, và cả hai
/// phải ghi cùng một định dạng — nếu mỗi bản tự định nghĩa, một bản sẽ ghi timestamp
/// lệch định dạng và so sánh chuỗi trong SQL (`>` cho `next_attempt_at`) sẽ hỏng.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}
