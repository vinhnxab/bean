//! Dung `TelegramChannel` va cac pho bien nho: typing, gui tin, skill nhap.
//!
//! # Bat bien
//!
//! * `typing` phai **dung** khi run ket thuc — bot Telegram de lai dau ba cham neu quen.
//! * Tin > 4096 ky tu phai tach **o ranh gioi dong/ky tu an toan** (muc 13).
//! * Skill nhap chi bat khi nguoi dung duyet (M15).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bean_skills::is_valid_draft_id;
use bean_types::RunId;
use secrecy::{ExposeSecret, SecretString};
use teloxide::Bot;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use bean_types::config::TelegramConfig;

use super::targets::{ConfirmTarget, DraftTarget, RunTarget};
use super::text_rate::{ChatRateLimiter, UpdateDedup, split_text};
use super::transport::TeloxideTransport;
use super::types::{
    TelegramButton, TelegramError, TelegramKeyboard, TelegramResult, TelegramTransport,
};
use super::{MAX_MESSAGE_CHARS, TYPING_INTERVAL, UPDATE_DEDUP_CAPACITY};

/// Telegram channel implementing the core `Channel` contract.
pub struct TelegramChannel {
    pub(super) transport: Arc<dyn TelegramTransport>,
    pub(super) allowed_user_ids: HashSet<i64>,
    pub(super) rate_limiter: Mutex<ChatRateLimiter>,
    pub(super) dedup: Mutex<UpdateDedup>,
    pub(super) runs: Mutex<HashMap<RunId, RunTarget>>,
    pub(super) confirms: Mutex<HashMap<String, ConfirmTarget>>,
    pub(super) drafts: Mutex<HashMap<String, DraftTarget>>,
    pub(super) typing_tasks: Mutex<HashMap<RunId, CancellationToken>>,
}

impl TelegramChannel {
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

    pub(super) fn run_target(&self, run_id: &RunId) -> Option<RunTarget> {
        self.runs.lock().ok()?.get(run_id).cloned()
    }

    pub(super) fn remove_run(&self, run_id: &RunId) -> Option<RunTarget> {
        self.runs.lock().ok()?.remove(run_id)
    }

    pub(super) fn remember_run(&self, run_id: &RunId, target: RunTarget) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(run_id.clone(), target);
        }
    }

    pub(super) async fn send_chat_text(&self, chat_id: i64, text: &str) {
        for part in split_text(text, MAX_MESSAGE_CHARS) {
            if let Err(error) = self.transport.send_text(chat_id, &part).await {
                tracing::warn!(chat_id, error = %error, "gửi text Telegram thất bại");
            }
        }
    }

    pub(super) async fn send_skill_draft(
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

    pub(super) async fn start_typing(
        &self,
        run_id: &RunId,
        chat_id: i64,
        shutdown: &CancellationToken,
    ) {
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

    pub(super) fn stop_typing(&self, run_id: &RunId) {
        if let Ok(mut tasks) = self.typing_tasks.lock()
            && let Some(task) = tasks.remove(run_id)
        {
            task.cancel();
        }
    }

    pub(super) fn stop_all_typing(&self) {
        let tasks = self
            .typing_tasks
            .lock()
            .map(|mut tasks| std::mem::take(&mut *tasks))
            .unwrap_or_default();
        for (_, task) in tasks {
            task.cancel();
        }
    }

    pub(super) async fn answer_callback(&self, callback_id: &str, text: Option<&str>) {
        if let Err(error) = self.transport.answer_callback(callback_id, text).await {
            tracing::warn!(callback_id, error = %error, "trả lời callback Telegram thất bại");
        }
    }
}
