//! Tool web M7: `web_fetch` và `web_search` (agents.md mục 7.3, 15.5).
//!
//! Mọi output từ Internet là dữ liệu không tin cậy: tool bật `ToolCtx::untrusted_seen` và
//! bọc kết quả bằng [`crate::untrusted::wrap`]. `web_fetch` dùng [`SafeHttpClient`]; các URL
//! trong kết quả search chỉ được tải tiếp nếu model gọi lại qua client này.
//!
//! # Module (tách theo trách nhiệm)
//! - `fetch` — tool `web_fetch` + helpers bọc untrusted
//! - `search` — tool `web_search` + client/provider tìm kiếm
//! - `tests` — test tích hợp cả hai tool (`#[cfg(test)]`)

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bean_tools::{Tool, ToolCtx, ToolError, TypedTool};
use bean_types::Risk;
use bean_types::config::MARKETING_READ_TAG;
use bean_types::config::{WebSearchConfig, WebSearchProvider};
use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;

use crate::ssrf::{DEFAULT_MAX_BODY_BYTES, SafeHttpClient, SsrfError};

/// Trần ký tự cho một lần gọi `web_fetch`, để cả metadata + thẻ untrusted vẫn dưới
/// ngưỡng 20.000 ký tự mà agent loop áp cho mọi tool.
pub const MAX_FETCH_OUTPUT_CHARS: usize = 15_000;
/// Giới hạn mặc định của `web_fetch`.
pub const DEFAULT_FETCH_CHARS: usize = 8_000;
/// Ngân sách ký tự cho kết quả `web_search` trước khi bọc untrusted.
pub const MAX_SEARCH_OUTPUT_CHARS: usize = 14_000;
/// Trần cứng sau khi escape thẻ untrusted, thấp hơn ngưỡng 20.000 của agent loop.
const MAX_WRAPPED_OUTPUT_CHARS: usize = 19_000;

mod fetch;
mod search;

#[cfg(test)]
mod tests;

pub use fetch::{WebFetchParams, web_fetch};
pub use search::{SearchConfigError, WebSearchParams, web_search};
