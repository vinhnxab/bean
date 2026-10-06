//! Xu ly tin nhan va callback quyet dinh tu nguoi dung.
//!
//! # Cho phep nguoi la (khong tien)
//!
//! Chi callback tu dung `user_id` da duoc cap phep moi duoc chap nhan (muc 13). Neu bo
//! qua rang buoc nay, bat ky ai cung co ID bot deu co the **duyet hanh dong nhanh cong**
//! thay user.

use bean_core::{Decision, Incoming, Router};
use tokio_util::sync::CancellationToken;

use std::time::Instant;

use super::channel_core::TelegramChannel;
use super::targets::{CallbackAction, RunTarget, parse_callback};
use super::types::{TelegramCallback, TelegramMessage};

impl TelegramChannel {
    pub(super) async fn handle_message(
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

    pub(super) async fn handle_callback(&self, router: &Router, callback: TelegramCallback) {
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
}
