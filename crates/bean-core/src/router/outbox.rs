//! Worker retry cho bảng `outbox` (agents.md mục 10, 14).
//!
//! # Vì sao tách khỏi [`super::Router`]
//!
//! Router trước đây ôm cả vòng đời run, confirm, slash command **và** việc thử lại
//! outbox trong một khối lớn. "Quét outbox đến hạn → gửi qua channel đã đăng ký →
//! backoff khi lỗi" là một trách nhiệm độc lập (SRP), nên được gom vào `OutboxWorker`.
//! Router chỉ còn hai method mỏng giao việc, giữ nguyên API công khai cho `bean`/test.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use super::{OUTBOX_BATCH_SIZE, RouterError, RouterInner, now_rfc3339, read_lock};

/// Trách nhiệm duy nhất: gửi lại các `Outbound` đã lưu trong bảng `outbox`.
///
/// Giữ `Weak<RouterInner>` (không phải `Arc`) để worker nền **không** giữ Router sống
/// sau khi tiến trình shutdown — đúng hành vi `Router::start_outbox_worker` cũ.
pub(super) struct OutboxWorker {
    inner: Weak<RouterInner>,
}

impl OutboxWorker {
    /// Tạo worker trỏ tới hạt nhân Router hiện có.
    pub(super) fn new(inner: &Arc<RouterInner>) -> Self {
        Self {
            inner: Arc::downgrade(inner),
        }
    }

    /// Khởi động vòng lặp thử lại **đúng một lần** cho mỗi tiến trình.
    ///
    /// Cờ `outbox_started` dùng compare-and-swap nên gọi nhiều lần là no-op.
    ///
    /// # Errors
    /// [`RouterError::NoRuntime`] khi không có Tokio runtime (worker cần `Handle`).
    pub(super) fn start(self: &Arc<Self>) -> Result<(), RouterError> {
        let Some(inner) = self.inner.upgrade() else {
            return Ok(());
        };
        if inner
            .outbox_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        let handle = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                inner.outbox_started.store(false, Ordering::Release);
                return Err(RouterError::NoRuntime);
            }
        };
        let weak = Arc::downgrade(&inner);
        let shutdown = inner.shutdown.clone();
        let interval = inner.options.outbox_poll_interval;
        drop(inner);
        handle.spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {
                        let Some(inner) = weak.upgrade() else { break };
                        let worker = OutboxWorker {
                            inner: Arc::downgrade(&inner),
                        };
                        if let Err(error) = worker.process_once().await {
                            tracing::warn!(error = %error, "worker outbox lỗi");
                        }
                    }
                }
            }
        });
        Ok(())
    }

    /// Xử lý một batch outbox đến hạn; trả số bản ghi đã thử.
    ///
    /// # Errors
    /// Lỗi store khi đọc/cập nhật bảng `outbox`.
    pub(super) async fn process_once(&self) -> Result<usize, RouterError> {
        let Some(inner) = self.inner.upgrade() else {
            return Ok(0);
        };
        let entries = inner
            .store
            .due_outbox(&now_rfc3339(), OUTBOX_BATCH_SIZE)
            .await?;
        let count = entries.len();
        for entry in entries {
            let adapter = read_lock(&inner.channels)?.get(&entry.channel).cloned();
            let result = match adapter {
                Some(adapter) => adapter.send(&entry.chat_id, entry.payload.clone()).await,
                None => Err(anyhow::anyhow!("channel chưa đăng ký: {}", entry.channel)),
            };
            match result {
                Ok(()) => inner.store.complete_outbox(entry.id).await?,
                Err(error) => {
                    let next_attempt_at = next_outbox_attempt(&inner, entry.attempts);
                    inner
                        .store
                        .retry_outbox(entry.id, &next_attempt_at, &error.to_string())
                        .await?;
                }
            }
        }
        Ok(count)
    }
}

/// Mốc thử lại kế tiếp bằng backoff mũ có trần (`outbox_max_delay`).
fn next_outbox_attempt(inner: &RouterInner, attempts: u32) -> String {
    let factor = 1_u32.checked_shl(attempts.min(16)).unwrap_or(u32::MAX);
    let delay = inner
        .options
        .outbox_base_delay
        .saturating_mul(factor)
        .min(inner.options.outbox_max_delay);
    let delay = chrono::Duration::from_std(delay).unwrap_or(chrono::Duration::MAX);
    (chrono::Utc::now() + delay).to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}
