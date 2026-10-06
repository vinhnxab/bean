//! Telegram adapter cho M12.
//!
//! Crate này chỉ là **adapter**: update của Telegram được dịch thành lời gọi `Router`,
//! và sự kiện của `Router` được dịch ngược thành yêu cầu Telegram. Không có logic
//! agent nào nằm ở đây.
//!
//! # Vì sao là thư mục chứ không phải một file
//!
//! File `telegram.rs` trước đây dài 1992 dòng và trộn **năm** việc không liên quan:
//! kiểu dữ liệu, quy tắc giới hạn của kênh, transport `teloxide`, bảng đích callback,
//! và vòng lặp polling. Hậu quả cụ thể: sửa quy tắc **allowlist user** (điều kiện an
//! toàn, mục 13) buộc phải mở file chứa cả vòng lặp `teloxide` và 777 dòng test.
//!
//! # Cách chia — theo ranh giới *loại công việc*
//!
//! ```text
//! telegram/
//! ├─ mod.rs           hằng số, khai báo module, bản đồ
//! ├─ types.rs         kiểu dữ liệu + trait `TelegramTransport` (seam cho test)
//! ├─ text_rate.rs     cat tin an toàn, giới hạn tần suất, chống trùng update
//! ├─ transport.rs     cầu nối teloxide (long polling)
//! ├─ targets.rs       bảng đích theo run_id + phân tích callback
//! ├─ channel_core.rs  dựng TelegramChannel, typing, gửi tin, skill nháp
//! ├─ handlers.rs      xử lý tin nhắn và callback quyết định
//! ├─ events.rs        RunEvent của Router → tin Telegram
//! ├─ polling.rs       vòng lặp polling + `impl Channel`
//! └─ tests.rs         test (không cần mạng, nhờ seam `TelegramTransport`)
//! ```
//!
//! # Bất biến phải giữ khi tách
//!
//! * **Allowlist user bắt buộc** — người lạ bị bỏ qua và ghi log, *không* trả lời để
//!   không lộ sự tồn tại của bot (mục 13). Ở `handlers.rs`.
//! * **Chỉ callback từ đúng `user_id` được cấp phép** mới được chấp nhận — nếu bỏ, ai có
//!   ID bot cũng duyệt được hành động thay user. Ở `handlers.rs`.
//! * **Cắt tin ở ranh giới an toàn** — Telegram giới hạn 4096 ký tự, cắt giữa ký tự UTF-8
//!   là panic. Ở `text_rate.rs`.
//!
//! * **Chống trùng update** — Telegram giao lại update khi mất ACK; chạy hai lần là
//!   thực thi tool hai lần. Ở `text_rate.rs`.

use std::time::Duration;

mod channel_core;
mod events;
mod handlers;
mod polling;
mod targets;
mod text_rate;
mod transport;
mod types;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Hằng số của kênh
// ---------------------------------------------------------------------------

const POLL_TIMEOUT_SECONDS: u32 = 10;
const TYPING_INTERVAL: Duration = Duration::from_secs(4);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const MAX_MESSAGE_CHARS: usize = 4_096;
const UPDATE_DEDUP_CAPACITY: usize = 1_024;

// ---------------------------------------------------------------------------
// Re-export — API công khai của crate
// ---------------------------------------------------------------------------

pub use self::channel_core::TelegramChannel;
pub use text_rate::{ChatRateLimiter, UpdateDedup, split_text};
pub use types::{
    TelegramButton, TelegramCallback, TelegramError, TelegramKeyboard, TelegramMessage,
    TelegramResult, TelegramTransport, TelegramUpdate, TelegramUpdateKind,
};
