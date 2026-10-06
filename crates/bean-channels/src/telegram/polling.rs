//! Vong lap long polling va `impl Channel`.
//!
//! `run()` phai tu ton tai **va tu dung** khi `CancellationToken` bi huy (muc 13).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use bean_core::{Channel, Router};
use bean_types::{Outbound, OutboundAction};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::channel_core::TelegramChannel;
use super::text_rate::split_text;
use super::types::{TelegramError, TelegramUpdate, TelegramUpdateKind};
use super::{MAX_MESSAGE_CHARS, MAX_RECONNECT_DELAY};

impl TelegramChannel {
    pub(super) async fn process_update(
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

    pub(super) async fn run_loop(
        &self,
        router: Arc<Router>,
        shutdown: CancellationToken,
    ) -> Result<()> {
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
#[async_trait::async_trait]
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
