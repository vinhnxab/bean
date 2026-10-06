//! Phát tin ra kênh: `notify`, outbox worker và `emit` sự kiện cho mọi
//! subscriber WebSocket/CLI/Telegram.

use super::*;

impl Router {
    /// Gửi outbound; lỗi/channel chưa đăng ký sẽ vào outbox.
    pub async fn notify(
        &self,
        channel: &str,
        chat_id: &str,
        out: Outbound,
    ) -> Result<(), RouterError> {
        let adapter = read_lock(&self.inner.channels)?.get(channel).cloned();
        let mut delivered = false;
        if let Some(adapter) = adapter {
            match adapter.send(chat_id, out.clone()).await {
                Ok(()) => delivered = true,
                Err(error) => {
                    tracing::warn!(channel, error = %error, "gửi outbound lỗi; lưu outbox");
                }
            }
        }
        if !delivered {
            self.inner
                .store
                .enqueue_outbound(channel, chat_id, &out, &now_rfc3339())
                .await?;
        }
        Ok(())
    }

    /// Dựng [`AlertSink`] gửi cảnh báo M23 tới kênh chính, hoặc `None` nếu chưa cấu hình.
    ///
    /// Trả `None` khi `alert_channel`/`alert_chat_id` trống — đó là cấu hình "không gửi
    /// cảnh báo trực tiếp", và `Config::validate` đã chặn trạng thái bật nửa chừng.
    #[must_use]
    pub fn alert_sink(&self) -> Option<Arc<dyn AlertSink>> {
        let config = read_lock(&self.inner.config).ok()?;
        let channel = config.security_scan.alert_channel.trim();
        let chat_id = config.security_scan.alert_chat_id.trim();
        if channel.is_empty() || chat_id.is_empty() {
            return None;
        }
        Some(Arc::new(RouterAlertSink::new(
            Arc::new(self.clone()),
            channel.to_string(),
            chat_id.to_string(),
        )))
    }

    /// Khởi động worker retry outbox đúng một lần.
    ///
    /// Giao việc cho [`outbox::OutboxWorker`] — Router không còn ôm logic retry outbox.
    pub fn start_outbox_worker(self: &Arc<Self>) -> Result<(), RouterError> {
        Arc::new(outbox::OutboxWorker::new(&self.inner)).start()
    }

    /// Xử lý một batch outbox đến hạn (public để test không phụ thuộc wall clock).
    ///
    /// Giao việc cho [`outbox::OutboxWorker`].
    pub async fn process_outbox_once(&self) -> Result<usize, RouterError> {
        outbox::OutboxWorker::new(&self.inner).process_once().await
    }
}

impl Router {
    pub(super) fn emit(&self, event: RunEvent) {
        let _ = self.inner.events.send(event);
    }

    pub(super) fn emit_error(&self, queued: &QueuedRun, code: &str, message: &str) {
        self.emit(RunEvent::Error {
            session_id: queued.session_id,
            run_id: queued.run_id.clone(),
            code: code.into(),
            message: message.into(),
        });
    }
}
