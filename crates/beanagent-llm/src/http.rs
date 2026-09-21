//! Dựng `reqwest::Client` dùng chung cho các provider (agents.md mục 5, 15.6).
//!
//! reqwest 0.13 dùng **rustls** làm backend mặc định — không kéo OpenSSL, đúng yêu cầu
//! agents.md mục 3.1. Header nhạy cảm (API key) không bao giờ được log ở đây.

use std::time::Duration;

use reqwest::Client;

use crate::LlmError;

/// Timeout cho một lượt gọi provider (mặc định 5 phút — không có trường cấu hình riêng).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Trần ký tự khi nhúng body lỗi vào [`LlmError::HttpStatus`].
const MAX_ERROR_BODY_CHARS: usize = 500;

/// Tạo `reqwest::Client`: rustls, timeout, connect-timeout và User-Agent riêng.
///
/// # Errors
/// [`LlmError::Config`] khi client không dựng được (hiếm: lỗi khởi tạo TLS backend).
pub fn build_client(timeout: Duration) -> Result<Client, LlmError> {
    Client::builder()
        .user_agent(concat!("BeanAgent/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(timeout)
        .build()
        .map_err(|err| LlmError::Config(format!("không tạo được HTTP client: {err}")))
}

/// Chuyển lỗi `reqwest` thành [`LlmError`] (timeout là một loại riêng để retry biết đường).
#[must_use]
pub fn to_transport_error(err: reqwest::Error) -> LlmError {
    if err.is_timeout() {
        LlmError::Timeout
    } else {
        LlmError::Transport(err.to_string())
    }
}

/// Cắt body lỗi xuống mức an toàn để nhúng vào [`LlmError::HttpStatus`] (giữ ranh giới ký tự).
#[must_use]
pub fn truncate_body(text: &str) -> String {
    if text.chars().count() <= MAX_ERROR_BODY_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
    format!("{cut}… [đã cắt]")
}

/// Đọc header `Retry-After` (giây). Giá trị HTTP-date **không** được hỗ trợ — các provider
/// lớn (Anthropic, OpenAI) đều dùng số giây; giá trị lạ được ghi log rồi bỏ qua.
#[must_use]
pub fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let raw = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    match raw.trim().parse::<u64>() {
        Ok(seconds) => Some(Duration::from_secs(seconds)),
        Err(_) => {
            tracing::debug!(value = raw, "Retry-After không phải số giây — bỏ qua");
            None
        }
    }
}
