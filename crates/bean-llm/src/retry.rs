//! Thử lại có backoff cho lỗi tạm thời (agents.md mục 5).
//!
//! Chính sách (nguồn sự thật: [`LlmError::is_retryable`]):
//! * thử lại: 429, 5xx, lỗi mạng, timeout — **tối đa 3 lần**;
//! * delay luỹ thừa 1s → 2s → 4s (+ jitter ≤ 25%);
//! * 429 kèm `Retry-After` → dùng đúng giá trị đó thay cho backoff;
//! * mọi 4xx khác → trả lỗi ngay, không thử lại.
//!
//! Wrapper này **tách khỏi** `LlmProvider::chat`: mỗi lượt `chat` chỉ phát đúng một
//! request (dễ đo đếm trong test, không retry kép khi M17 thêm streaming).

use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

use crate::LlmError;

/// Số lần thử lại tối đa **sau** lần đầu (agents.md mục 5: "tối đa 3 lần").
pub const MAX_RETRIES: u32 = 3;

/// Delay gốc của backoff luỹ thừa: `1s * 2^attempt` trước jitter.
pub const BASE_DELAY: Duration = Duration::from_secs(1);

/// Jitter tối đa: delay thực nằm trong `[delay, delay * 1.25)`.
const MAX_JITTER_FRACTION: f64 = 0.25;

/// Chạy `op`, thử lại theo chính sách trên.
///
/// `base_delay` được truyền vào (thay vì hằng cứng) để test chạy nhanh với delay nhỏ.
pub async fn retry_with_backoff<T, F, Fut>(
    base_delay: Duration,
    max_retries: u32,
    mut op: F,
) -> Result<T, LlmError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, LlmError>>,
{
    let mut attempt: u32 = 0;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if !err.is_retryable() || attempt >= max_retries {
                    return Err(err);
                }
                let delay = err
                    .retry_after()
                    .unwrap_or_else(|| jitter(base_delay.saturating_mul(1u32 << attempt)));
                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries,
                    delay_ms = delay.as_millis() as u64,
                    error = %err,
                    "lỗi tạm thời khi gọi provider — thử lại"
                );
                attempt += 1;
                sleep(delay).await;
            }
        }
    }
}

/// Jitter dựa trên nano-giây đồng hồ hệ thống (không kéo thêm crate `rand`).
fn jitter(delay: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let factor = 1.0 + f64::from(nanos % 1000) / 1000.0 * MAX_JITTER_FRACTION;
    Duration::from_nanos((delay.as_nanos() as f64 * factor) as u64)
}
