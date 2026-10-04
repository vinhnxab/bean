//! Cầu nối cảnh báo chủ động của Router (M23).
//!
//! `RouterAlertSink` biến `AlertSink` của tool thành tin nhắn đi qua
//! [`Router::notify`], nên nó thuộc tầng kênh chứ không phải lõi điều phối.
//!
//! Cố ý **mọi** cảnh báo đều qua `notify`: lỗi gửi rơi vào bảng `outbox` và được
//! thử lại, nên không mất tin cảnh báo an ninh.

use std::sync::Arc;

use bean_tools::AlertSink;
use bean_types::{Alert, Outbound, OutboundKind};

use super::Router;

/// Cài bản [`AlertSink`] bọc quanh [`Router::notify`] (M23).
///
/// Giữ **một** đường gửi: mọi cảnh báo đi qua `notify` nên lỗi vẫn rơi vào outbox và được
/// thử lại — không mất tin cảnh báo an ninh.
pub(super) struct RouterAlertSink {
    router: Arc<Router>,
    channel: String,
    chat_id: String,
}

impl RouterAlertSink {
    /// Dựng cầu nối gửi cảnh báo về `channel`/`chat_id`.
    ///
    /// Trường để private ngoài module này nên chỉ [`Router::alert_sink`] dựng được —
    /// đảm bảo mọi `AlertSink` đều đi qua đúng một đường gửi.
    pub(super) fn new(router: Arc<Router>, channel: String, chat_id: String) -> Self {
        Self {
            router,
            channel,
            chat_id,
        }
    }
}

#[async_trait::async_trait]
impl AlertSink for RouterAlertSink {
    async fn send_alert(&self, alert: &Alert) -> Result<(), String> {
        let text = if alert.risks.is_empty() {
            format!("{}\n{}", alert.title, alert.summary)
        } else {
            format!(
                "{}\n{}\n- {}",
                alert.title,
                alert.summary,
                alert.risks.join("\n- ")
            )
        };
        // Ghi vào lịch sử trước để `message_id` có thật (adapter hiển thị và UI cần id này).
        let session = self
            .router
            .inner
            .store
            .ensure_session(&self.channel, &self.chat_id, &alert.title)
            .await
            .map_err(|error| error.to_string())?;
        let message_id = self
            .router
            .inner
            .store
            .append(
                session,
                bean_types::Message::assistant(Some(text.clone()), Vec::new()),
            )
            .await
            .map_err(|error| error.to_string())?;
        self.router
            .notify(
                &self.channel,
                &self.chat_id,
                Outbound {
                    session_id: session,
                    message_id,
                    text,
                    kind: OutboundKind::Notification,
                    action: None,
                },
            )
            .await
            .map_err(|error| error.to_string())
    }
}
