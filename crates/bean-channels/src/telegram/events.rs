//! Chuyen `RunEvent` cua Router thanh tin Telegram.
//!
//! # Bat bien quan trong
//!
//! `RunEvent` la **su kien cua Router**, khong phai cua ket noi: dong bot hay rot mang
//! khong duoc lam run bay. Handler nay chi *hien thi* (muc 10).

use bean_types::RunEvent;

use super::channel_core::TelegramChannel;
use super::targets::{ConfirmTarget, event_run_id, resolved_text};
use super::types::{TelegramButton, TelegramKeyboard};

impl TelegramChannel {
    pub(super) async fn handle_event(&self, event: RunEvent) {
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
}
