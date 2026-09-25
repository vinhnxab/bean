//! Telegram adapter for M12.
//!
//! This crate is an adapter only: Telegram updates are translated into
//! `Router` calls and Router events are translated back into Telegram requests.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use beanagent_core::{Channel, Decision, Incoming, Router};
use beanagent_skills::is_valid_draft_id;
use beanagent_types::config::TelegramConfig;
use beanagent_types::{ConfirmOutcome, Outbound, OutboundAction, RunEvent, RunId};
use secrecy::{ExposeSecret, SecretString};
use teloxide::prelude::*;
use teloxide::types::{
    AllowedUpdate, CallbackQueryId, ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup,
    MessageId, ReplyMarkup, Update, UpdateKind,
};
use teloxide::{ApiError, Bot, RequestError};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{interval, sleep};
use tokio_util::sync::CancellationToken;

const POLL_TIMEOUT_SECONDS: u32 = 10;
const TYPING_INTERVAL: Duration = Duration::from_secs(4);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const MAX_MESSAGE_CHARS: usize = 4_096;
const UPDATE_DEDUP_CAPACITY: usize = 1_024;

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

/// Split text for Telegram's 4096-unit limit without cutting UTF-8.
#[must_use]
pub fn split_text(text: &str, max_units: usize) -> Vec<String> {
    if text.is_empty() || max_units == 0 {
        return Vec::new();
    }
    if max_units == 1 {
        return text.chars().map(|ch| ch.to_string()).collect();
    }
    let mut parts = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut units = 0usize;
        let mut end = None;
        let mut boundary = None;
        for (relative, ch) in text[start..].char_indices() {
            let next = units.saturating_add(ch.len_utf16());
            if next > max_units {
                end = Some(start + relative);
                break;
            }
            units = next;
            let byte_end = start + relative + ch.len_utf8();
            if ch.is_whitespace() {
                boundary = Some(byte_end);
            }
            if units == max_units {
                end = Some(byte_end);
                break;
            }
        }
        let Some(end) = end else {
            parts.push(text[start..].to_owned());
            break;
        };
        let cut = boundary
            .filter(|value| *value > start && *value < end)
            .unwrap_or(end);
        parts.push(text[start..cut].to_owned());
        start = cut;
    }
    parts
}

/// Bounded, deterministic per-chat rate limiter.
#[derive(Debug)]
pub struct ChatRateLimiter {
    limit: usize,
    window: Duration,
    entries: HashMap<i64, VecDeque<Instant>>,
}

impl ChatRateLimiter {
    /// Construct a limiter for a per-minute limit.
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self {
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
            window: Duration::from_secs(60),
            entries: HashMap::new(),
        }
    }

    /// Check and record one event at an explicit instant.
    pub fn allow(&mut self, chat_id: i64, now: Instant) -> bool {
        let cutoff = now.checked_sub(self.window);
        let entries = self.entries.entry(chat_id).or_default();
        if let Some(cutoff) = cutoff {
            while entries.front().is_some_and(|seen| *seen <= cutoff) {
                entries.pop_front();
            }
        }
        if entries.len() >= self.limit {
            return false;
        }
        entries.push_back(now);
        true
    }
}

/// Bounded update-id deduplicator.
#[derive(Debug)]
pub struct UpdateDedup {
    seen: HashSet<u32>,
    order: VecDeque<u32>,
    capacity: usize,
}

impl UpdateDedup {
    /// Construct a deduplicator with a fixed memory bound.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    /// Return `true` only for the first occurrence of an update id.
    pub fn accept(&mut self, update_id: u32) -> bool {
        if !self.seen.insert(update_id) {
            return false;
        }
        self.order.push_back(update_id);
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

struct PollingState {
    offset: i32,
    pending: VecDeque<Update>,
}

struct TeloxideTransport {
    bot: Bot,
    state: AsyncMutex<PollingState>,
}

impl TeloxideTransport {
    fn new(bot: Bot) -> Self {
        Self {
            bot,
            state: AsyncMutex::new(PollingState {
                offset: 0,
                pending: VecDeque::new(),
            }),
        }
    }
}

fn map_request_error(error: RequestError) -> TelegramError {
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

fn convert_update(update: Update) -> Option<TelegramUpdate> {
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

fn to_teloxide_keyboard(keyboard: TelegramKeyboard) -> InlineKeyboardMarkup {
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

#[derive(Clone, Debug)]
struct RunTarget {
    chat_id: i64,
    user_id: String,
}

#[derive(Clone, Debug)]
struct DraftTarget {
    chat_id: i64,
    message_id: i32,
    user_id: String,
}

#[derive(Clone, Debug)]
struct ConfirmTarget {
    chat_id: i64,
    message_id: i32,
    user_id: String,
    allow_session: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallbackAction {
    Confirm(Decision),
    ApproveDraft,
    RejectDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCallback {
    action: CallbackAction,
    id: String,
}

fn parse_callback(data: &str) -> Option<ParsedCallback> {
    if data.len() > 64 {
        return None;
    }
    let (action, id) = data.rsplit_once(':')?;
    if is_valid_confirm_id(id) {
        let decision = match action {
            "a" => Decision::Allow,
            "s" => Decision::AllowInSession,
            "d" => Decision::Deny,
            _ => return None,
        };
        return Some(ParsedCallback {
            action: CallbackAction::Confirm(decision),
            id: id.to_owned(),
        });
    }
    if is_valid_draft_id(id) {
        let action = match action {
            "skill:a" => CallbackAction::ApproveDraft,
            "skill:r" => CallbackAction::RejectDraft,
            _ => return None,
        };
        return Some(ParsedCallback {
            action,
            id: id.to_owned(),
        });
    }
    None
}

fn is_valid_confirm_id(value: &str) -> bool {
    value.len() == 40
        && value.starts_with("confirm_")
        && value
            .as_bytes()
            .get(8..)
            .is_some_and(|suffix| suffix.len() == 32 && suffix.iter().all(u8::is_ascii_hexdigit))
}

fn event_run_id(event: &RunEvent) -> &RunId {
    match event {
        RunEvent::Queued { run_id, .. }
        | RunEvent::Text { run_id, .. }
        | RunEvent::TextDelta { run_id, .. }
        | RunEvent::ToolStart { run_id, .. }
        | RunEvent::ToolEnd { run_id, .. }
        | RunEvent::ConfirmRequest { run_id, .. }
        | RunEvent::ConfirmResolved { run_id, .. }
        | RunEvent::Final { run_id, .. }
        | RunEvent::Error { run_id, .. } => run_id,
    }
}

fn resolved_text(outcome: ConfirmOutcome) -> &'static str {
    match outcome {
        ConfirmOutcome::Allowed => "Đã cho phép",
        ConfirmOutcome::Denied => "Từ chối",
        ConfirmOutcome::Expired => "Hết hạn",
    }
}

/// Telegram channel implementing the core `Channel` contract.
pub struct TelegramChannel {
    transport: Arc<dyn TelegramTransport>,
    allowed_user_ids: HashSet<i64>,
    rate_limiter: Mutex<ChatRateLimiter>,
    dedup: Mutex<UpdateDedup>,
    runs: Mutex<HashMap<RunId, RunTarget>>,
    confirms: Mutex<HashMap<String, ConfirmTarget>>,
    drafts: Mutex<HashMap<String, DraftTarget>>,
    typing_tasks: Mutex<HashMap<RunId, CancellationToken>>,
}

impl TelegramChannel {
    /// Build a production channel from a secret bot token.
    #[must_use]
    pub fn new(token: SecretString, config: TelegramConfig) -> Self {
        let bot = Bot::new(token.expose_secret().to_owned());
        Self::with_transport(Arc::new(TeloxideTransport::new(bot)), config)
    }

    /// Build a channel around a transport, primarily useful for tests.
    #[must_use]
    pub fn with_transport(transport: Arc<dyn TelegramTransport>, config: TelegramConfig) -> Self {
        Self {
            transport,
            allowed_user_ids: config.allowed_user_ids.into_iter().collect(),
            rate_limiter: Mutex::new(ChatRateLimiter::new(config.rate_limit_per_minute)),
            dedup: Mutex::new(UpdateDedup::new(UPDATE_DEDUP_CAPACITY)),
            runs: Mutex::new(HashMap::new()),
            confirms: Mutex::new(HashMap::new()),
            drafts: Mutex::new(HashMap::new()),
            typing_tasks: Mutex::new(HashMap::new()),
        }
    }

    fn run_target(&self, run_id: &RunId) -> Option<RunTarget> {
        self.runs.lock().ok()?.get(run_id).cloned()
    }

    fn remove_run(&self, run_id: &RunId) -> Option<RunTarget> {
        self.runs.lock().ok()?.remove(run_id)
    }

    fn remember_run(&self, run_id: &RunId, target: RunTarget) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(run_id.clone(), target);
        }
    }

    async fn send_chat_text(&self, chat_id: i64, text: &str) {
        for part in split_text(text, MAX_MESSAGE_CHARS) {
            if let Err(error) = self.transport.send_text(chat_id, &part).await {
                tracing::warn!(chat_id, error = %error, "gửi text Telegram thất bại");
            }
        }
    }

    async fn send_skill_draft(
        &self,
        chat_id: i64,
        text: &str,
        draft_id: &str,
        actor: &str,
    ) -> TelegramResult<()> {
        if !is_valid_draft_id(draft_id) {
            return Err(TelegramError::Transport(
                "ID skill nháp không hợp lệ".into(),
            ));
        }
        let keyboard = TelegramKeyboard {
            rows: vec![vec![
                TelegramButton {
                    text: "Duyệt".into(),
                    callback_data: format!("skill:a:{draft_id}"),
                },
                TelegramButton {
                    text: "Bỏ".into(),
                    callback_data: format!("skill:r:{draft_id}"),
                },
            ]],
        };
        let message_id = self
            .transport
            .send_confirmation(chat_id, text, keyboard)
            .await?;
        if let Ok(mut drafts) = self.drafts.lock() {
            drafts.insert(
                draft_id.to_string(),
                DraftTarget {
                    chat_id,
                    message_id,
                    user_id: actor.to_string(),
                },
            );
        }
        Ok(())
    }

    async fn start_typing(&self, run_id: &RunId, chat_id: i64, shutdown: &CancellationToken) {
        let cancel = shutdown.child_token();
        if let Ok(mut tasks) = self.typing_tasks.lock()
            && let Some(previous) = tasks.insert(run_id.clone(), cancel.clone())
        {
            previous.cancel();
        }
        let transport = Arc::clone(&self.transport);
        let run_key = run_id.clone();
        tokio::spawn(async move {
            let mut ticker = interval(TYPING_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = ticker.tick() => {
                        let result = tokio::select! {
                            _ = cancel.cancelled() => break,
                            result = transport.send_typing(chat_id) => result,
                        };
                        if let Err(error) = result {
                            tracing::warn!(run_id = %run_key, chat_id, error = %error,
                                "gửi typing Telegram thất bại");
                        }
                    }
                }
            }
        });
    }

    fn stop_typing(&self, run_id: &RunId) {
        if let Ok(mut tasks) = self.typing_tasks.lock()
            && let Some(task) = tasks.remove(run_id)
        {
            task.cancel();
        }
    }

    fn stop_all_typing(&self) {
        let tasks = self
            .typing_tasks
            .lock()
            .map(|mut tasks| std::mem::take(&mut *tasks))
            .unwrap_or_default();
        for (_, task) in tasks {
            task.cancel();
        }
    }

    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) {
        if let Err(error) = self.transport.answer_callback(callback_id, text).await {
            tracing::warn!(callback_id, error = %error, "trả lời callback Telegram thất bại");
        }
    }

    async fn handle_message(
        &self,
        router: &Router,
        message: TelegramMessage,
        shutdown: &CancellationToken,
    ) {
        if !self.allowed_user_ids.contains(&message.user_id) {
            tracing::warn!(
                user_id = message.user_id,
                chat_id = message.chat_id,
                "bỏ qua Telegram user ngoài allowlist"
            );
            return;
        }
        let now = Instant::now();
        let allowed = self
            .rate_limiter
            .lock()
            .map(|mut limiter| limiter.allow(message.chat_id, now))
            .unwrap_or(false);
        if !allowed {
            tracing::warn!(
                chat_id = message.chat_id,
                "bỏ qua Telegram message vượt rate limit"
            );
            return;
        }
        let text = message.text.trim();
        if text.is_empty() {
            return;
        }
        let actor = format!("telegram:{}", message.user_id);
        let incoming = Incoming::new("telegram", message.chat_id.to_string(), actor.clone(), text);
        match router.submit(incoming).await {
            Ok(run_id) => {
                self.remember_run(
                    &run_id,
                    RunTarget {
                        chat_id: message.chat_id,
                        user_id: actor,
                    },
                );
                self.start_typing(&run_id, message.chat_id, shutdown).await;
            }
            Err(error) => tracing::warn!(chat_id = message.chat_id, error = %error,
                "Router từ chối Telegram message"),
        }
    }

    async fn handle_callback(&self, router: &Router, callback: TelegramCallback) {
        let actor = format!("telegram:{}", callback.user_id);
        if !self.allowed_user_ids.contains(&callback.user_id) {
            tracing::warn!(
                user_id = callback.user_id,
                "bỏ qua Telegram callback ngoài allowlist"
            );
            return;
        }
        let now = Instant::now();
        let rate_key = callback.chat_id.unwrap_or(callback.user_id);
        let allowed = self
            .rate_limiter
            .lock()
            .map(|mut limiter| limiter.allow(rate_key, now))
            .unwrap_or(false);
        if !allowed {
            self.answer_callback(&callback.callback_id, Some("Bạn gửi quá nhiều yêu cầu"))
                .await;
            return;
        }
        let Some(parsed) = parse_callback(&callback.data) else {
            self.answer_callback(&callback.callback_id, Some("Yêu cầu không hợp lệ"))
                .await;
            return;
        };
        match parsed.action {
            CallbackAction::Confirm(decision) => {
                let target = self
                    .confirms
                    .lock()
                    .ok()
                    .and_then(|confirms| confirms.get(&parsed.id).cloned());
                let Some(target) = target else {
                    self.answer_callback(&callback.callback_id, Some("Yêu cầu đã hết hạn"))
                        .await;
                    return;
                };
                if callback.chat_id != Some(target.chat_id)
                    || callback.message_id != Some(target.message_id)
                    || target.user_id != actor
                {
                    self.answer_callback(&callback.callback_id, Some("Yêu cầu không hợp lệ"))
                        .await;
                    return;
                }
                if matches!(decision, Decision::AllowInSession) && !target.allow_session {
                    self.answer_callback(
                        &callback.callback_id,
                        Some("Tác vụ này không cho phép trong phiên"),
                    )
                    .await;
                    return;
                }
                if let Err(error) = router.resolve_confirm(&parsed.id, decision, &actor).await {
                    tracing::debug!(confirm_id = %parsed.id, error = %error,
                        "callback Telegram không resolve được confirm");
                    self.answer_callback(&callback.callback_id, Some("Yêu cầu đã hết hạn"))
                        .await;
                } else {
                    self.answer_callback(&callback.callback_id, Some("Đã ghi nhận"))
                        .await;
                }
            }
            CallbackAction::ApproveDraft | CallbackAction::RejectDraft => {
                let target = self
                    .drafts
                    .lock()
                    .ok()
                    .and_then(|drafts| drafts.get(&parsed.id).cloned());
                let Some(target) = target else {
                    self.answer_callback(&callback.callback_id, Some("Đề xuất đã hết hạn"))
                        .await;
                    return;
                };
                if callback.chat_id != Some(target.chat_id)
                    || callback.message_id != Some(target.message_id)
                    || target.user_id != actor
                {
                    self.answer_callback(&callback.callback_id, Some("Yêu cầu không hợp lệ"))
                        .await;
                    return;
                }
                let claimed = self
                    .drafts
                    .lock()
                    .ok()
                    .and_then(|mut drafts| drafts.remove(&parsed.id));
                if claimed.is_none() {
                    self.answer_callback(&callback.callback_id, Some("Đề xuất đã hết hạn"))
                        .await;
                    return;
                }
                let approve = matches!(parsed.action, CallbackAction::ApproveDraft);
                let result = if approve {
                    router.approve_draft(&parsed.id, &actor).await
                } else {
                    router.reject_draft(&parsed.id, &actor).await
                };
                match result {
                    Ok(_) => {
                        let text = if approve {
                            "Đã duyệt skill"
                        } else {
                            "Đã bỏ đề xuất"
                        };
                        self.answer_callback(&callback.callback_id, Some(text))
                            .await;
                        if let Err(error) = self
                            .transport
                            .edit_text(target.chat_id, target.message_id, text, true)
                            .await
                        {
                            tracing::warn!(draft_id = %parsed.id, error = %error,
                                "cập nhật thông báo draft Telegram thất bại");
                        }
                    }
                    Err(error) => {
                        tracing::debug!(draft_id = %parsed.id, error = %error,
                            "callback Telegram không xử lý được draft");
                        self.answer_callback(
                            &callback.callback_id,
                            Some("Đề xuất không còn hợp lệ"),
                        )
                        .await;
                    }
                }
            }
        }
    }

    async fn handle_event(&self, event: RunEvent) {
        let run_id = event_run_id(&event).clone();
        match event {
            RunEvent::Queued { position, .. } => {
                if let Some(target) = self.run_target(&run_id) {
                    self.send_chat_text(
                        target.chat_id,
                        &format!("Đang xếp hàng (vị trí {position})..."),
                    )
                    .await;
                }
            }
            RunEvent::Text { text, .. } => {
                if let Some(target) = self.run_target(&run_id) {
                    self.send_chat_text(target.chat_id, &text).await;
                }
            }
            // Streaming token chỉ dành cho Web UI. Telegram chờ Final để tránh gửi
            // hàng trăm tin nhắn và vẫn nhận đúng một tin đầy đủ.
            RunEvent::TextDelta { .. } => {}
            RunEvent::ToolStart { tool, summary, .. } => {
                if let Some(target) = self.run_target(&run_id) {
                    self.send_chat_text(target.chat_id, &format!("🔧 {tool}: {summary}"))
                        .await;
                }
            }
            RunEvent::ToolEnd {
                tool,
                ok,
                output_preview,
                ..
            } => {
                if let Some(target) = self.run_target(&run_id) {
                    let status = if ok { "xong" } else { "lỗi" };
                    let text = if output_preview.is_empty() {
                        format!("{tool}: {status}")
                    } else {
                        format!("{tool}: {status}\n{output_preview}")
                    };
                    self.send_chat_text(target.chat_id, &text).await;
                }
            }
            RunEvent::ConfirmRequest {
                confirm_id,
                prompt,
                allow_session_option,
                ..
            } => {
                let Some(target) = self.run_target(&run_id) else {
                    return;
                };
                let confirm = confirm_id.to_string();
                let mut buttons = vec![TelegramButton {
                    text: "Cho phép".into(),
                    callback_data: format!("a:{confirm}"),
                }];
                if allow_session_option {
                    buttons.push(TelegramButton {
                        text: "Cho phép trong phiên".into(),
                        callback_data: format!("s:{confirm}"),
                    });
                }
                buttons.push(TelegramButton {
                    text: "Từ chối".into(),
                    callback_data: format!("d:{confirm}"),
                });
                let keyboard = TelegramKeyboard {
                    rows: vec![buttons],
                };
                match self
                    .transport
                    .send_confirmation(target.chat_id, &prompt, keyboard)
                    .await
                {
                    Ok(message_id) => {
                        if let Ok(mut confirms) = self.confirms.lock() {
                            confirms.insert(
                                confirm,
                                ConfirmTarget {
                                    chat_id: target.chat_id,
                                    message_id,
                                    user_id: target.user_id,
                                    allow_session: allow_session_option,
                                },
                            );
                        }
                    }
                    Err(error) => tracing::warn!(run_id = %run_id, error = %error,
                        "gửi xác nhận Telegram thất bại"),
                }
            }
            RunEvent::ConfirmResolved {
                confirm_id,
                outcome,
                ..
            } => {
                let target = self
                    .confirms
                    .lock()
                    .ok()
                    .and_then(|mut confirms| confirms.remove(&confirm_id.to_string()));
                if let Some(target) = target
                    && let Err(error) = self
                        .transport
                        .edit_text(
                            target.chat_id,
                            target.message_id,
                            resolved_text(outcome),
                            true,
                        )
                        .await
                {
                    tracing::warn!(confirm_id = %confirm_id, error = %error,
                        "cập nhật xác nhận Telegram thất bại");
                }
            }
            RunEvent::Final { text, .. } => {
                if let Some(target) = self.remove_run(&run_id) {
                    self.send_chat_text(target.chat_id, &text).await;
                }
                self.stop_typing(&run_id);
            }
            RunEvent::Error { code, message, .. } => {
                if let Some(target) = self.remove_run(&run_id) {
                    self.send_chat_text(target.chat_id, &format!("Lỗi {code}: {message}"))
                        .await;
                }
                self.stop_typing(&run_id);
            }
        }
    }

    async fn process_update(
        &self,
        router: &Router,
        update: TelegramUpdate,
        shutdown: &CancellationToken,
    ) {
        let accepted = self
            .dedup
            .lock()
            .map(|mut dedup| dedup.accept(update.update_id))
            .unwrap_or(false);
        if !accepted {
            return;
        }
        match update.kind {
            TelegramUpdateKind::Message(message) => {
                self.handle_message(router, message, shutdown).await;
            }
            TelegramUpdateKind::Callback(callback) => {
                self.handle_callback(router, callback).await;
            }
        }
    }

    async fn run_loop(&self, router: Arc<Router>, shutdown: CancellationToken) -> Result<()> {
        let mut reconnect_delay = Duration::from_secs(1);
        loop {
            let prepared = tokio::select! {
                _ = shutdown.cancelled() => {
                    self.stop_all_typing();
                    return Ok(());
                }
                result = self.transport.prepare() => result,
            };
            match prepared {
                Ok(()) => break,
                Err(TelegramError::Conflict) => {
                    tracing::error!(
                        "Telegram 409: bot token đang được một instance khác sử dụng; dừng instance này"
                    );
                    return Err(anyhow!(
                        "Telegram 409: bot token đang được một instance khác sử dụng; dừng instance này"
                    ));
                }
                Err(TelegramError::RetryAfter(seconds)) => {
                    let delay = Duration::from_secs(u64::from(seconds));
                    tracing::warn!(
                        seconds,
                        "Telegram yêu cầu tạm dừng trước khi khởi tạo lại polling"
                    );
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            self.stop_all_typing();
                            return Ok(());
                        }
                        _ = sleep(delay) => {}
                    }
                }
                Err(TelegramError::Network) => {
                    tracing::warn!("Telegram chưa kết nối được; sẽ thử lại");
                    tokio::select! {
                        _ = shutdown.cancelled() => {
                            self.stop_all_typing();
                            return Ok(());
                        }
                        _ = sleep(reconnect_delay) => {}
                    }
                    reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                }
                Err(error) => {
                    return Err(anyhow!("không khởi tạo được Telegram polling: {error}"));
                }
            }
        }
        let mut events = router.events();
        reconnect_delay = Duration::from_secs(1);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    self.stop_all_typing();
                    return Ok(());
                }
                event = router.recv_event(&mut events) => {
                    if let Some(event) = event {
                        self.handle_event(event).await;
                    }
                }
                update = self.transport.next_update() => {
                    match update {
                        Ok(Some(update)) => {
                            reconnect_delay = Duration::from_secs(1);
                            self.process_update(&router, update, &shutdown).await;
                        }
                        Ok(None) => {}
                        Err(TelegramError::Conflict) => {
                            tracing::error!(
                                "Telegram 409: bot token đang được một instance khác sử dụng; dừng instance này"
                            );
                            return Err(anyhow!(
                                "Telegram 409: bot token đang được một instance khác sử dụng; dừng instance này"
                            ));
                        }
                        Err(TelegramError::RetryAfter(seconds)) => {
                            let delay = Duration::from_secs(u64::from(seconds));
                            tracing::warn!(seconds, "Telegram yêu cầu tạm dừng trước khi polling lại");
                            tokio::select! {
                                _ = shutdown.cancelled() => {
                                    self.stop_all_typing();
                                    return Ok(());
                                }
                                _ = sleep(delay) => {}
                            }
                        }
                        Err(error) => {
                            tracing::warn!(error = %error, "Telegram polling lỗi; sẽ thử lại");
                            tokio::select! {
                                _ = shutdown.cancelled() => {
                                    self.stop_all_typing();
                                    return Ok(());
                                }
                                _ = sleep(reconnect_delay) => {}
                            }
                            reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn run(&self, router: Arc<Router>, shutdown: CancellationToken) -> Result<()> {
        self.run_loop(router, shutdown).await
    }

    async fn send(&self, chat_id: &str, out: Outbound) -> Result<()> {
        let chat_id = chat_id
            .parse::<i64>()
            .map_err(|_| anyhow!("chat_id Telegram không hợp lệ: {chat_id}"))?;
        if let Some(OutboundAction::SkillDraft { id, actor, .. }) = out.action
            && actor.starts_with("telegram:")
        {
            self.send_skill_draft(chat_id, &out.text, &id, &actor)
                .await
                .map_err(|error| anyhow!("gửi skill draft Telegram thất bại: {error}"))?;
            return Ok(());
        }
        for part in split_text(&out.text, MAX_MESSAGE_CHARS) {
            self.transport
                .send_text(chat_id, &part)
                .await
                .map_err(|error| anyhow!("gửi outbound Telegram thất bại: {error}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use beanagent_core::MemoryStore;
    use beanagent_core::router::{RouterDeps, RouterOptions};
    use beanagent_llm::FakeProvider;
    use beanagent_security::CapWorkspace;
    use beanagent_skills::{NewSkillDraft, SkillDraftKind};
    use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
    use beanagent_types::{Config, LlmResponse, Risk, ToolCall, ToolSpec};
    use tokio::time::{Duration, timeout};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Command {
        Text {
            chat_id: i64,
            text: String,
        },
        Confirmation {
            chat_id: i64,
            text: String,
            keyboard: TelegramKeyboard,
            message_id: i32,
        },
        Typing(i64),
        Edit {
            chat_id: i64,
            message_id: i32,
            text: String,
            remove_keyboard: bool,
        },
        Callback {
            id: String,
            text: Option<String>,
        },
    }

    #[derive(Default)]
    struct MockTransport {
        updates: AsyncMutex<VecDeque<TelegramResult<Option<TelegramUpdate>>>>,
        commands: AsyncMutex<Vec<Command>>,
        next_message_id: AtomicUsize,
        prepared: AtomicUsize,
    }

    impl MockTransport {
        fn new() -> Self {
            Self::default()
        }

        async fn push_update(&self, update: TelegramUpdate) {
            self.updates.lock().await.push_back(Ok(Some(update)));
        }

        async fn push_error(&self, error: TelegramError) {
            self.updates.lock().await.push_back(Err(error));
        }

        async fn commands(&self) -> Vec<Command> {
            self.commands.lock().await.clone()
        }
    }

    #[async_trait]
    impl TelegramTransport for MockTransport {
        async fn prepare(&self) -> TelegramResult<()> {
            self.prepared.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn next_update(&self) -> TelegramResult<Option<TelegramUpdate>> {
            match self.updates.lock().await.pop_front() {
                Some(result) => result,
                None => Ok(None),
            }
        }

        async fn send_text(&self, chat_id: i64, text: &str) -> TelegramResult<i32> {
            self.commands.lock().await.push(Command::Text {
                chat_id,
                text: text.to_owned(),
            });
            Ok(self.next_message_id.fetch_add(1, Ordering::SeqCst) as i32 + 100)
        }

        async fn send_confirmation(
            &self,
            chat_id: i64,
            text: &str,
            keyboard: TelegramKeyboard,
        ) -> TelegramResult<i32> {
            self.commands.lock().await.push(Command::Confirmation {
                chat_id,
                text: text.to_owned(),
                keyboard,
                message_id: self.next_message_id.fetch_add(1, Ordering::SeqCst) as i32 + 100,
            });
            Ok(self.next_message_id.load(Ordering::SeqCst) as i32 + 99)
        }

        async fn send_typing(&self, chat_id: i64) -> TelegramResult<()> {
            self.commands.lock().await.push(Command::Typing(chat_id));
            Ok(())
        }

        async fn edit_text(
            &self,
            chat_id: i64,
            message_id: i32,
            text: &str,
            remove_keyboard: bool,
        ) -> TelegramResult<()> {
            self.commands.lock().await.push(Command::Edit {
                chat_id,
                message_id,
                text: text.to_owned(),
                remove_keyboard,
            });
            Ok(())
        }

        async fn answer_callback(
            &self,
            callback_id: &str,
            text: Option<&str>,
        ) -> TelegramResult<()> {
            self.commands.lock().await.push(Command::Callback {
                id: callback_id.to_owned(),
                text: text.map(str::to_owned),
            });
            Ok(())
        }
    }

    struct RiskTool {
        risk: Risk,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Tool for RiskTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(
                "risk_tool",
                "test risk",
                serde_json::json!({"type": "object"}),
            )
        }

        fn risk(&self, _args: &serde_json::Value) -> Risk {
            self.risk
        }

        async fn call(
            &self,
            _ctx: &ToolCtx,
            _args: serde_json::Value,
        ) -> Result<String, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("done".into())
        }
    }

    fn telegram_config() -> TelegramConfig {
        TelegramConfig {
            enabled: true,
            token_env: "TEST_TELEGRAM_TOKEN".into(),
            allowed_user_ids: vec![42],
            rate_limit_per_minute: 20,
        }
    }

    async fn make_router(
        responses: Vec<LlmResponse>,
        tools: Vec<Arc<dyn Tool>>,
        options: RouterOptions,
    ) -> (tempfile::TempDir, Arc<Router>) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Arc::new(CapWorkspace::open(temp.path().to_path_buf()).unwrap());
        let mut registry = ToolRegistry::with_workspace(workspace);
        for tool in tools {
            registry.register(tool).unwrap();
        }
        let mut config = Config::default();
        config.agent.allowed_users = vec!["telegram:42".into()];
        config.agent.max_steps = 4;
        let router = Arc::new(Router::with_options(
            RouterDeps {
                config,
                store: Arc::new(MemoryStore::new()),
                registry: Arc::new(registry),
                llm: Arc::new(FakeProvider::new(responses)),
                audit: None,
                skills_index: String::new(),
                skills: None,
            },
            options,
        ));
        (temp, router)
    }

    fn message_update(update_id: u32, user_id: i64, text: &str) -> TelegramUpdate {
        TelegramUpdate {
            update_id,
            kind: TelegramUpdateKind::Message(TelegramMessage {
                chat_id: 100,
                message_id: 1,
                user_id,
                text: text.to_owned(),
            }),
        }
    }

    fn callback_update(
        update_id: u32,
        user_id: i64,
        data: &str,
        message_id: i32,
    ) -> TelegramUpdate {
        TelegramUpdate {
            update_id,
            kind: TelegramUpdateKind::Callback(TelegramCallback {
                callback_id: format!("callback-{update_id}"),
                user_id,
                chat_id: Some(100),
                message_id: Some(message_id),
                data: data.to_owned(),
            }),
        }
    }

    async fn wait_command(
        transport: &MockTransport,
        predicate: impl Fn(&Command) -> bool,
    ) -> Command {
        let result = timeout(Duration::from_secs(3), async {
            loop {
                if let Some(command) = transport.commands().await.into_iter().find(&predicate) {
                    return command;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        match result {
            Ok(command) => command,
            Err(_) => panic!(
                "phải nhận Telegram command; commands={:?}",
                transport.commands().await
            ),
        }
    }

    fn spawn_channel(
        channel: Arc<TelegramChannel>,
        router: Arc<Router>,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<Result<()>> {
        tokio::spawn(async move { channel.run(router, shutdown).await })
    }

    fn confirmation_buttons(command: &Command) -> Vec<TelegramButton> {
        match command {
            Command::Confirmation { keyboard, .. } => {
                keyboard.rows.first().cloned().unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    fn confirmation_message_id(command: &Command) -> i32 {
        match command {
            Command::Confirmation { message_id, .. } => *message_id,
            _ => 0,
        }
    }

    #[test]
    fn split_text_preserves_utf8_and_utf16_limit() {
        let text = format!("{}{}", "Việt Nam 🦀 ".repeat(1_000), "😀".repeat(10));
        let parts = split_text(&text, 4_096);
        assert!(
            parts
                .iter()
                .all(|part| part.encode_utf16().count() <= 4_096)
        );
        assert_eq!(parts.concat(), text);
    }

    #[test]
    fn split_text_handles_limit_smaller_than_one_emoji() {
        let text = "a😀b";
        let parts = split_text(text, 1);
        assert_eq!(parts.concat(), text);
        assert_eq!(parts, vec!["a", "😀", "b"]);
    }

    #[test]
    fn split_text_cuts_before_scalar_that_exceeds_utf16_limit() {
        let text = "😀😀😀";
        let parts = split_text(text, 3);
        assert_eq!(parts.concat(), text);
        assert!(parts.iter().all(|part| part.encode_utf16().count() <= 3));
        assert_eq!(parts, vec!["😀", "😀", "😀"]);
    }

    #[test]
    fn split_text_prefers_safe_boundaries() {
        let text = "một hai ba bốn năm";
        assert_eq!(split_text(text, 8), vec!["một hai ", "ba bốn ", "năm"]);
    }

    #[test]
    fn rate_limit_is_per_chat_and_sliding() {
        let mut limiter = ChatRateLimiter::new(2);
        let start = Instant::now();
        assert!(limiter.allow(1, start));
        assert!(limiter.allow(1, start + Duration::from_secs(1)));
        assert!(!limiter.allow(1, start + Duration::from_secs(2)));
        assert!(limiter.allow(2, start + Duration::from_secs(2)));
        assert!(limiter.allow(1, start + Duration::from_secs(61)));
    }

    #[test]
    fn update_dedup_is_bounded() {
        let mut dedup = UpdateDedup::new(2);
        assert!(dedup.accept(1));
        assert!(!dedup.accept(1));
        assert!(dedup.accept(2));
        assert!(dedup.accept(3));
        assert!(dedup.accept(1));
    }

    #[test]
    fn callback_data_is_short_and_minimal() {
        let confirm_id = format!("confirm_{}", "a".repeat(32));
        let parsed = parse_callback(&format!("a:{confirm_id}"));
        assert_eq!(
            parsed,
            Some(ParsedCallback {
                action: CallbackAction::Confirm(Decision::Allow),
                id: confirm_id.clone(),
            })
        );
        assert_eq!(format!("a:{confirm_id}").len(), 42);
        assert_eq!(parse_callback("a:secret:extra"), None);
    }

    #[tokio::test]
    async fn allowlist_blocks_unknown_user_without_reply() {
        let transport = Arc::new(MockTransport::new());
        transport
            .push_update(message_update(1, 999, "xin chào"))
            .await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let (_temp, router) = make_router(
            vec![LlmResponse::text_only("không được gửi")],
            vec![],
            RouterOptions::default(),
        )
        .await;
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router, shutdown.clone());
        tokio::time::sleep(Duration::from_millis(50)).await;
        shutdown.cancel();
        task.await.unwrap().unwrap();
        assert!(transport.commands().await.is_empty());
    }

    #[tokio::test]
    async fn dedup_does_not_submit_same_update_twice() {
        let transport = Arc::new(MockTransport::new());
        let update = message_update(7, 42, "chỉ một lần");
        transport.push_update(update.clone()).await;
        transport.push_update(update).await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let (_temp, router) = make_router(
            vec![LlmResponse::text_only("xong")],
            vec![],
            RouterOptions::default(),
        )
        .await;
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router, shutdown.clone());
        wait_command(
            &transport,
            |command| matches!(command, Command::Text { text, .. } if text == "xong"),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown.cancel();
        task.await.unwrap().unwrap();
        let finals = transport
            .commands()
            .await
            .into_iter()
            .filter(|command| matches!(command, Command::Text { text, .. } if text == "xong"))
            .count();
        assert_eq!(finals, 1);
    }

    #[tokio::test]
    async fn rate_limited_message_is_not_submitted() {
        let mut config = telegram_config();
        config.rate_limit_per_minute = 1;
        let transport = Arc::new(MockTransport::new());
        transport.push_update(message_update(1, 42, "một")).await;
        transport.push_update(message_update(2, 42, "hai")).await;
        let channel = Arc::new(TelegramChannel::with_transport(transport.clone(), config));
        let (_temp, router) = make_router(
            vec![
                LlmResponse::text_only("trả lời một"),
                LlmResponse::text_only("trả lời hai"),
            ],
            vec![],
            RouterOptions::default(),
        )
        .await;
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router, shutdown.clone());
        wait_command(
            &transport,
            |command| matches!(command, Command::Text { text, .. } if text == "trả lời một"),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        shutdown.cancel();
        task.await.unwrap().unwrap();
        assert!(
            !transport.commands().await.iter().any(
                |command| matches!(command, Command::Text { text, .. } if text == "trả lời hai")
            )
        );
    }

    #[tokio::test]
    async fn confirm_button_resolves_through_router() {
        let tool = Arc::new(RiskTool {
            risk: Risk::Confirm,
            calls: AtomicUsize::new(0),
        });
        let responses = vec![
            LlmResponse::with_tool_calls(vec![ToolCall::new(
                "c1",
                "risk_tool",
                serde_json::json!({}),
            )]),
            LlmResponse::text_only("đã xác nhận"),
        ];
        let (_temp, router) =
            make_router(responses, vec![tool.clone()], RouterOptions::default()).await;
        let transport = Arc::new(MockTransport::new());
        transport
            .push_update(message_update(1, 42, "chạy tool"))
            .await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router.clone(), shutdown.clone());
        let confirmation = wait_command(&transport, |command| {
            matches!(command, Command::Confirmation { .. })
        })
        .await;
        let buttons = confirmation_buttons(&confirmation);
        assert_eq!(buttons.len(), 3);
        assert!(
            buttons
                .iter()
                .all(|button| button.callback_data.len() <= 64)
        );
        assert!(buttons[0].callback_data.starts_with('a'));
        transport
            .push_update(callback_update(
                2,
                42,
                &buttons[0].callback_data,
                confirmation_message_id(&confirmation),
            ))
            .await;
        wait_command(
            &transport,
            |command| matches!(command, Command::Text { text, .. } if text == "đã xác nhận"),
        )
        .await;
        shutdown.cancel();
        task.await.unwrap().unwrap();
        assert_eq!(tool.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dangerous_confirmation_has_no_session_button() {
        let tool = Arc::new(RiskTool {
            risk: Risk::Dangerous,
            calls: AtomicUsize::new(0),
        });
        let (_temp, router) = make_router(
            vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
                "c1",
                "risk_tool",
                serde_json::json!({}),
            )])],
            vec![tool],
            RouterOptions {
                confirm_timeout: Duration::from_millis(50),
                ..RouterOptions::default()
            },
        )
        .await;
        let transport = Arc::new(MockTransport::new());
        transport
            .push_update(message_update(1, 42, "nguy hiểm"))
            .await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel, router, shutdown.clone());
        let confirmation = wait_command(&transport, |command| {
            matches!(command, Command::Confirmation { .. })
        })
        .await;
        let buttons = confirmation_buttons(&confirmation);
        assert_eq!(buttons.len(), 2);
        assert!(
            buttons
                .iter()
                .all(|button| button.text != "Cho phép trong phiên")
        );
        shutdown.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn callback_from_unknown_user_does_not_resolve_confirm() {
        let tool = Arc::new(RiskTool {
            risk: Risk::Confirm,
            calls: AtomicUsize::new(0),
        });
        let (_temp, router) = make_router(
            vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
                "c1",
                "risk_tool",
                serde_json::json!({}),
            )])],
            vec![tool.clone()],
            RouterOptions {
                confirm_timeout: Duration::from_secs(30),
                ..RouterOptions::default()
            },
        )
        .await;
        let transport = Arc::new(MockTransport::new());
        transport
            .push_update(message_update(1, 42, "chạy tool"))
            .await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router.clone(), shutdown.clone());
        let confirmation = wait_command(&transport, |command| {
            matches!(command, Command::Confirmation { .. })
        })
        .await;
        let data = confirmation_buttons(&confirmation)[0].callback_data.clone();
        transport
            .push_update(callback_update(
                2,
                999,
                &data,
                confirmation_message_id(&confirmation),
            ))
            .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !transport
                .commands()
                .await
                .iter()
                .any(|command| matches!(command, Command::Callback { .. }))
        );
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
        assert_eq!(router.snapshot().pending_confirms.len(), 1);
        shutdown.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn expired_confirm_is_edited_to_expired_text() {
        let tool = Arc::new(RiskTool {
            risk: Risk::Confirm,
            calls: AtomicUsize::new(0),
        });
        let (_temp, router) = make_router(
            vec![LlmResponse::with_tool_calls(vec![ToolCall::new(
                "c1",
                "risk_tool",
                serde_json::json!({}),
            )])],
            vec![tool.clone()],
            RouterOptions {
                confirm_timeout: Duration::from_millis(30),
                ..RouterOptions::default()
            },
        )
        .await;
        let transport = Arc::new(MockTransport::new());
        transport
            .push_update(message_update(1, 42, "chạy tool"))
            .await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router, shutdown.clone());
        wait_command(&transport, |command| {
            matches!(command, Command::Edit { text, remove_keyboard: true, .. } if text == "Hết hạn")
        })
        .await;
        assert_eq!(tool.calls.load(Ordering::SeqCst), 0);
        shutdown.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn polling_retries_network_error_and_reports_conflict() {
        let transport = Arc::new(MockTransport::new());
        transport.push_error(TelegramError::Network).await;
        transport
            .push_update(message_update(1, 42, "sau reconnect"))
            .await;
        let channel = Arc::new(TelegramChannel::with_transport(
            transport.clone(),
            telegram_config(),
        ));
        let (_temp, router) = make_router(
            vec![LlmResponse::text_only("đã nhận")],
            vec![],
            RouterOptions::default(),
        )
        .await;
        let shutdown = CancellationToken::new();
        let task = spawn_channel(channel.clone(), router, shutdown.clone());
        wait_command(
            &transport,
            |command| matches!(command, Command::Text { text, .. } if text == "đã nhận"),
        )
        .await;
        shutdown.cancel();
        task.await.unwrap().unwrap();
        assert_eq!(transport.prepared.load(Ordering::SeqCst), 1);

        let conflict_transport = Arc::new(MockTransport::new());
        conflict_transport.push_error(TelegramError::Conflict).await;
        let conflict_channel = Arc::new(TelegramChannel::with_transport(
            conflict_transport,
            telegram_config(),
        ));
        let (_temp, router) = make_router(
            vec![LlmResponse::text_only("x")],
            vec![],
            RouterOptions::default(),
        )
        .await;
        let error = conflict_channel
            .run(router, CancellationToken::new())
            .await
            .expect_err("409 phải là fatal");
        assert!(error.to_string().contains("409"));
    }
    #[tokio::test]
    async fn skill_draft_inline_button_approves_for_exact_allowed_user() {
        let temp = tempfile::tempdir().unwrap();
        let skills_root = temp.path().join("skills");
        let catalog = beanagent_skills::SkillCatalog::load_with_paths(
            std::slice::from_ref(&skills_root),
            skills_root.clone(),
            skills_root.join("_drafts"),
        );
        let draft = catalog
            .create_draft(NewSkillDraft {
                name: "release-checklist".into(),
                kind: SkillDraftKind::New,
                description: "Dùng trước khi phát hành.".into(),
                body: "# Steps\n1. Chạy test.".into(),
                reason: "Quy trình lặp lại.".into(),
                source_session_id: 1,
                source_channel: "telegram".into(),
                source_chat_id: "100".into(),
                created_at: "2026-09-25T10:00:00Z".into(),
            })
            .unwrap();
        let workspace_path = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_path).unwrap();
        let workspace = Arc::new(CapWorkspace::open(workspace_path).unwrap());
        let mut config = Config::default();
        config.agent.allowed_users = vec!["telegram:42".into()];
        let router = Arc::new(Router::new(RouterDeps {
            config,
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(ToolRegistry::with_workspace(workspace)),
            llm: Arc::new(FakeProvider::echo()),
            audit: None,
            skills_index: catalog.index(),
            skills: Some(catalog.clone()),
        }));
        let transport = Arc::new(MockTransport::new());
        let mut telegram_config = telegram_config();
        telegram_config.allowed_user_ids.push(43);
        let channel = TelegramChannel::with_transport(transport.clone(), telegram_config);
        channel
            .send(
                "100",
                Outbound {
                    session_id: beanagent_types::SessionId::new(1),
                    message_id: 1,
                    text: format!("Đề xuất `{}`", draft.name),
                    kind: beanagent_types::OutboundKind::Notification,
                    action: Some(OutboundAction::SkillDraft {
                        id: draft.id.clone(),
                        name: draft.name.clone(),
                        actor: "telegram:42".into(),
                    }),
                },
            )
            .await
            .unwrap();
        let command = wait_command(&transport, |command| {
            matches!(command, Command::Confirmation { .. })
        })
        .await;
        let message_id = confirmation_message_id(&command);
        let buttons = confirmation_buttons(&command);
        assert_eq!(buttons[0].callback_data, format!("skill:a:{}", draft.id));
        assert_eq!(buttons[1].callback_data, format!("skill:r:{}", draft.id));

        let wrong_user = callback_update(98, 43, &buttons[0].callback_data, message_id);
        channel
            .process_update(&router, wrong_user, &CancellationToken::new())
            .await;
        wait_command(&transport, |command| {
            matches!(command, Command::Callback { text: Some(text), .. } if text == "Yêu cầu không hợp lệ")
        })
        .await;
        assert!(catalog.get("release-checklist").is_err());

        let update = callback_update(99, 42, &buttons[0].callback_data, message_id);
        channel
            .process_update(&router, update, &CancellationToken::new())
            .await;
        wait_command(&transport, |command| {
            matches!(command, Command::Edit { text, remove_keyboard: true, .. } if text == "Đã duyệt skill")
        })
        .await;
        assert!(catalog.get("release-checklist").is_ok());
    }
}
