//! # BeanAgent-channels
//!
//! Adapter cho các kênh ngoài CLI/web (agents.md mục 13): mỗi kênh implement `trait Channel`
//! của `BeanAgent-core` và **mỏng** — nhận input, gọi `Router::submit`, hiển thị `RunEvent`,
//! không chứa logic agent.
//!
//! * **M12**: Telegram bằng `teloxide 0.17` (long polling qua transport,
//!   `default-features = false` + `rustls` để không kéo OpenSSL). Allowlist user id là
//!   **bắt buộc**; người lạ bị bỏ qua và ghi log (không trả lời để không lộ sự tồn tại của bot).
//! * **M17 (tuỳ chọn)**: Discord (`serenity`/`twilight`), Slack (`slack-morphism`).
#![forbid(unsafe_code)]

pub mod telegram;

pub use telegram::{
    ChatRateLimiter, TelegramButton, TelegramCallback, TelegramChannel, TelegramError,
    TelegramKeyboard, TelegramMessage, TelegramResult, TelegramTransport, TelegramUpdate,
    TelegramUpdateKind, UpdateDedup, split_text,
};
