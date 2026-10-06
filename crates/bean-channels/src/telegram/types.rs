//! Kieu du lieu va trait `TelegramTransport` — ranh gioi cua adapter.
//!
//! # Vai tro
//!
//! Day la **seam giua Telegram va core**: moi thu adapter doc deu la du lieu thuan
//! (khong phai kieu cua `teloxide`), va moi thao tac gui deu qua trait
//! `TelegramTransport`. Nho vay test co the kiem tra logic allowlist va confirm
//! **khong can bot token, khong can mang**.
//!
//! # Vi sao tach rieng
//!
//! Cac kieu nay la **hop dong** ma ca ben trong va ben ngoai deu noi den. Giu chung
//! trong mot file de dang bi sua o cho mot nguoi va lam hong nguoi khac, va de doc
//! hieu phai luot qua phan dinh nghia cua Telegram.

use async_trait::async_trait;
use thiserror::Error;

/// Result alias used by the transport boundary.
pub type TelegramResult<T> = std::result::Result<T, TelegramError>;

/// Error returned by the Telegram transport boundary.
#[derive(Debug, Error)]
pub enum TelegramError {
    /// Telegram returned a non-conflict API error.
    #[error("Telegram API lỗi: {0}")]
    Api(String),
    /// Another poller owns the same bot token.
    #[error("Telegram 409: bot token đang được một instance khác sử dụng")]
    Conflict,
    /// Network failure while talking to Telegram.
    #[error("lỗi mạng Telegram (không log chi tiết có thể chứa token)")]
    Network,
    /// Telegram asked the client to retry after a number of seconds.
    #[error("Telegram yêu cầu thử lại sau {0} giây")]
    RetryAfter(u32),
    /// Local transport state is unavailable.
    #[error("lỗi transport Telegram: {0}")]
    Transport(String),
}

/// A text message reduced to the fields needed by the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelegramMessage {
    /// Telegram chat id.
    pub chat_id: i64,
    /// Telegram message id.
    pub message_id: i32,
    /// Telegram user id of the sender.
    pub user_id: i64,
    /// Message text.
    pub text: String,
}

/// A callback query reduced to the fields needed by the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelegramCallback {
    /// Telegram callback query id.
    pub callback_id: String,
    /// Telegram user id of the caller.
    pub user_id: i64,
    /// Chat containing the button, when supplied by Telegram.
    pub chat_id: Option<i64>,
    /// Message containing the button, when supplied by Telegram.
    pub message_id: Option<i32>,
    /// Opaque callback data.
    pub data: String,
}

/// Update payload understood by the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TelegramUpdateKind {
    /// A text message.
    Message(TelegramMessage),
    /// A callback query.
    Callback(TelegramCallback),
}

/// A converted Telegram update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelegramUpdate {
    /// Monotonic Telegram update id.
    pub update_id: u32,
    /// Supported update payload.
    pub kind: TelegramUpdateKind,
}

/// One button in a confirmation keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelegramButton {
    /// Visible label.
    pub text: String,
    /// Value sent back by Telegram.
    pub callback_data: String,
}

/// Inline keyboard rows used by the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelegramKeyboard {
    /// Button rows.
    pub rows: Vec<Vec<TelegramButton>>,
}

/// Semantic operations needed by `TelegramChannel`.
///
/// Tests can implement this trait with a mock and never need a bot token.
#[async_trait]
pub trait TelegramTransport: Send + Sync {
    /// Prepare polling state before the first request.
    async fn prepare(&self) -> TelegramResult<()> {
        Ok(())
    }

    /// Fetch the next update, or `None` after an empty long poll.
    async fn next_update(&self) -> TelegramResult<Option<TelegramUpdate>>;

    /// Send plain text and return Telegram's message id.
    async fn send_text(&self, chat_id: i64, text: &str) -> TelegramResult<i32>;

    /// Send a confirmation and return the id of the keyboard message.
    async fn send_confirmation(
        &self,
        chat_id: i64,
        text: &str,
        keyboard: TelegramKeyboard,
    ) -> TelegramResult<i32>;

    /// Refresh the typing indicator.
    async fn send_typing(&self, chat_id: i64) -> TelegramResult<()>;

    /// Edit a message, optionally removing its inline keyboard.
    async fn edit_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        remove_keyboard: bool,
    ) -> TelegramResult<()>;

    /// Stop Telegram's callback spinner.
    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> TelegramResult<()>;
}
