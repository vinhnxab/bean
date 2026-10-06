//! Cầu nối với Telegram API qua `teloxide` (dài polling).
//!
//! # Vì sao có trait
//!
//! `TelegramTransport` là **seam** để test: test không được gọi mạng thật (chậm, tốn
//! tiền, và cần token). Nhờ đây mọi test chạy với transport giả và kiểm tra đúng logic
//! allowlist mà không chạm Telegram.

use std::collections::VecDeque;

use async_trait::async_trait;
use teloxide::prelude::*;
use teloxide::types::{
    AllowedUpdate, CallbackQueryId, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup,
    MessageId, ReplyMarkup, Update, UpdateKind,
};
use teloxide::{ApiError, Bot, RequestError};
use tokio::sync::Mutex as AsyncMutex;

use super::text_rate::split_text;
use super::types::{
    TelegramCallback, TelegramError, TelegramKeyboard, TelegramMessage, TelegramResult,
    TelegramTransport, TelegramUpdate, TelegramUpdateKind,
};
use super::{MAX_MESSAGE_CHARS, POLL_TIMEOUT_SECONDS};
pub(super) struct PollingState {
    offset: i32,
    pending: VecDeque<Update>,
}

pub(super) struct TeloxideTransport {
    bot: Bot,
    state: AsyncMutex<PollingState>,
}

impl TeloxideTransport {
    pub(super) fn new(bot: Bot) -> Self {
        Self {
            bot,
            state: AsyncMutex::new(PollingState {
                offset: 0,
                pending: VecDeque::new(),
            }),
        }
    }
}

pub(super) fn map_request_error(error: RequestError) -> TelegramError {
    match error {
        RequestError::Api(ApiError::TerminatedByOtherGetUpdates) => TelegramError::Conflict,
        RequestError::Api(error) => TelegramError::Api(error.to_string()),
        RequestError::RetryAfter(seconds) => TelegramError::RetryAfter(seconds.seconds()),
        // `reqwest::Error` can include the request URL, whose path contains the bot
        // token. Never retain or log its Display/Debug representation.
        RequestError::Network(_) => TelegramError::Network,
        other => TelegramError::Api(other.to_string()),
    }
}

pub(super) fn convert_update(update: Update) -> Option<TelegramUpdate> {
    let update_id = update.id.0;
    match update.kind {
        UpdateKind::Message(message) => {
            let text = message.text()?.to_owned();
            let user_id = i64::try_from(message.from?.id.0).ok()?;
            Some(TelegramUpdate {
                update_id,
                kind: TelegramUpdateKind::Message(TelegramMessage {
                    chat_id: message.chat.id.0,
                    message_id: message.id.0,
                    user_id,
                    text,
                }),
            })
        }
        UpdateKind::CallbackQuery(callback) => {
            let user_id = i64::try_from(callback.from.id.0).ok()?;
            let (chat_id, message_id) = callback
                .message
                .as_ref()
                .map(|message| (Some(message.chat().id.0), Some(message.id().0)))
                .unwrap_or((None, None));
            Some(TelegramUpdate {
                update_id,
                kind: TelegramUpdateKind::Callback(TelegramCallback {
                    callback_id: callback.id.0,
                    user_id,
                    chat_id,
                    message_id,
                    data: callback.data.unwrap_or_default(),
                }),
            })
        }
        _ => None,
    }
}

pub(super) fn to_teloxide_keyboard(keyboard: TelegramKeyboard) -> InlineKeyboardMarkup {
    let rows = keyboard
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|button| InlineKeyboardButton::callback(button.text, button.callback_data))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(rows)
}

#[async_trait]
impl TelegramTransport for TeloxideTransport {
    async fn prepare(&self) -> TelegramResult<()> {
        let info = self
            .bot
            .get_webhook_info()
            .send()
            .await
            .map_err(map_request_error)?;
        if info.url.is_some() {
            self.bot
                .delete_webhook()
                .send()
                .await
                .map_err(map_request_error)?;
        }
        Ok(())
    }

    async fn next_update(&self) -> TelegramResult<Option<TelegramUpdate>> {
        let mut state = self.state.lock().await;
        while let Some(update) = state.pending.pop_front() {
            if let Some(update) = convert_update(update) {
                return Ok(Some(update));
            }
        }
        let updates = self
            .bot
            .get_updates()
            .offset(state.offset)
            .timeout(POLL_TIMEOUT_SECONDS)
            .allowed_updates(vec![AllowedUpdate::Message, AllowedUpdate::CallbackQuery])
            .send()
            .await
            .map_err(map_request_error)?;
        if let Some(last) = updates.last() {
            let next_offset = i32::try_from(last.id.0)
                .ok()
                .and_then(|id| id.checked_add(1))
                .ok_or_else(|| TelegramError::Transport("update_id quá lớn".into()))?;
            state.offset = next_offset;
        }
        state.pending.extend(updates);
        while let Some(update) = state.pending.pop_front() {
            if let Some(update) = convert_update(update) {
                return Ok(Some(update));
            }
        }
        Ok(None)
    }

    async fn send_text(&self, chat_id: i64, text: &str) -> TelegramResult<i32> {
        self.bot
            .send_message(ChatId(chat_id), text)
            .send()
            .await
            .map(|message| message.id.0)
            .map_err(map_request_error)
    }

    async fn send_confirmation(
        &self,
        chat_id: i64,
        text: &str,
        keyboard: TelegramKeyboard,
    ) -> TelegramResult<i32> {
        let parts = split_text(text, MAX_MESSAGE_CHARS);
        if parts.is_empty() {
            return Err(TelegramError::Transport("nội dung xác nhận rỗng".into()));
        }
        let last = parts.len() - 1;
        let keyboard = to_teloxide_keyboard(keyboard);
        for (index, part) in parts.iter().enumerate() {
            let message_id = if index == last {
                self.bot
                    .send_message(ChatId(chat_id), part)
                    .reply_markup(ReplyMarkup::InlineKeyboard(keyboard.clone()))
                    .send()
                    .await
                    .map_err(map_request_error)?
                    .id
                    .0
            } else {
                self.send_text(chat_id, part).await?
            };
            if index == last {
                return Ok(message_id);
            }
        }
        Err(TelegramError::Transport("không gửi được xác nhận".into()))
    }

    async fn send_typing(&self, chat_id: i64) -> TelegramResult<()> {
        // A fresh action immediately refreshes the indicator; subsequent refreshes
        // are driven by the run-specific periodic task in `TelegramChannel`.
        self.bot
            .send_chat_action(ChatId(chat_id), ChatAction::Typing)
            .send()
            .await
            .map(|_| ())
            .map_err(map_request_error)
    }

    async fn edit_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        remove_keyboard: bool,
    ) -> TelegramResult<()> {
        let request = self
            .bot
            .edit_message_text(ChatId(chat_id), MessageId(message_id), text);
        let request = if remove_keyboard {
            request.reply_markup(InlineKeyboardMarkup::default())
        } else {
            request
        };
        request.send().await.map(|_| ()).map_err(map_request_error)
    }

    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> TelegramResult<()> {
        let request = self
            .bot
            .answer_callback_query(CallbackQueryId::from(callback_id.to_owned()));
        let request = if let Some(value) = text {
            request.text(value.to_owned())
        } else {
            request
        };
        request.send().await.map(|_| ()).map_err(map_request_error)
    }
}
