//! Probe "kiểm tra kết nối" dùng chung — **một nguồn sự thật** cho `bean doctor
//! --online` và `POST /api/settings/test/{target}` (M29 mục 8.1).
//!
//! # Bất biến
//!
//! * Không bao giờ in giá trị secret; mọi lỗi mạng phải `without_url()` — URL Telegram
//!   chứa token, in nguyên lỗi là lộ token (mục 15.6).
//! * `probe_search` dựng client **no_proxy** + tắt redirect, giống hệt `SearchClient`
//!   của `bean-security` (K29): ping qua proxy hỏng sẽ báo sai so với hành vi thật
//!   của tool `web_search`. LLM/ElevenLabs/Telegram thì đi qua biến môi trường proxy
//!   như mọi request khác của tiến trình.
//! * Kết quả là [`ProbeOutcome`] (mức + thông điệp đã redact + gợi ý) — người gọi tự
//!   quyết in ra CLI hay trả JSON; probe không có side-effect ngoài request mạng.

use std::time::Duration;

use bean_llm::{ChatRequest, LlmProvider};
use bean_types::Message;
use bean_types::config::WebSearchProvider;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;

/// Mức kết quả của một lần probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeLevel {
    /// Kiểm tra đạt.
    Ok,
    /// Đạt nhưng đáng lưu ý (hết hạn mức, quá tải, trạng thái lạ).
    Warn,
    /// Thất bại — key sai, không tới được server, cấu hình thiếu.
    Error,
}

impl ProbeLevel {
    /// Chuỗi máy cho JSON API (`ok` | `warn` | `error`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Kết quả một lần probe: mức, thông điệp đã redact và gợi ý xử lý.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// Mức kết quả.
    pub level: ProbeLevel,
    /// Thông điệp cho người dùng — không chứa secret.
    pub message: String,
    /// Gợi ý xử lý khi có vấn đề.
    pub hint: Option<String>,
}

impl ProbeOutcome {
    /// Kiểm tra đạt.
    #[must_use]
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            level: ProbeLevel::Ok,
            message: message.into(),
            hint: None,
        }
    }

    /// Đạt nhưng đáng lưu ý.
    #[must_use]
    pub fn warn(message: impl Into<String>) -> Self {
        Self {
            level: ProbeLevel::Warn,
            message: message.into(),
            hint: None,
        }
    }

    /// Thất bại.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: ProbeLevel::Error,
            message: message.into(),
            hint: None,
        }
    }

    /// Gắn gợi ý xử lý.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// HTTP client cho probe ra ngoài (LLM, ElevenLabs, Telegram): đi qua biến môi
/// trường proxy như mọi request khác của tiến trình.
#[must_use]
pub fn online_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("bean/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .ok()
}

/// Ping LLM bằng request nhỏ nhất (`max_tokens = 1`) — đủ để xác nhận endpoint,
/// key và model hợp lệ mà chi phí không đáng kể.
pub async fn probe_llm(
    provider: &dyn LlmProvider,
    provider_label: &str,
    model: &str,
) -> ProbeOutcome {
    let messages = [Message::user("ping")];
    let request = ChatRequest {
        system: "You are a health check.",
        messages: &messages,
        tools: &[],
        max_tokens: 1,
    };
    match provider.chat(request).await {
        Ok(response) => ProbeOutcome::ok(format!(
            "LLM {provider_label}/{model} phản hồi ({} in / {} out tokens)",
            response.usage.input_tokens, response.usage.output_tokens
        )),
        Err(error) => ProbeOutcome::error(format!("LLM ping thất bại ({provider_label}): {error}")),
    }
}

/// GET `{base_url}/v1/user` với header `xi-api-key` — endpoint rẻ nhất của ElevenLabs.
pub async fn probe_voice_api(base_url: &str, key: &SecretString) -> ProbeOutcome {
    let Some(client) = online_client() else {
        return ProbeOutcome::warn("voice api: không dựng được HTTP client");
    };
    let url = format!("{}/v1/user", base_url.trim_end_matches('/'));
    match client
        .get(&url)
        .header("xi-api-key", key.expose_secret())
        .send()
        .await
    {
        Ok(response) => match response.status().as_u16() {
            200..=299 => ProbeOutcome::ok("voice api: ElevenLabs chấp nhận key"),
            401 | 403 => ProbeOutcome::error("voice api: key bị từ chối (HTTP 401/403)")
                .with_hint("rotate key trên elevenlabs.io rồi `bean secret set`"),
            402 => ProbeOutcome::warn("voice api: HTTP 402 — hết hạn mức ElevenLabs (gói free)")
                .with_hint("đổi giọng trong voice vault free hoặc nâng gói"),
            status => ProbeOutcome::warn(format!("voice api: HTTP {status}")),
        },
        Err(error) => ProbeOutcome::error(format!(
            "voice api: không tới được ElevenLabs: {}",
            error.without_url()
        )),
    }
}

/// `getMe` của Telegram bot — URL chứa token nên mọi lỗi phải `without_url()`.
pub async fn probe_telegram(token: &SecretString) -> ProbeOutcome {
    let Some(client) = online_client() else {
        return ProbeOutcome::warn("telegram: không dựng được HTTP client");
    };
    // URL chứa token — KHÔNG BAO GIỜ in URL hay body; lỗi reqwest phải without_url().
    let url = format!(
        "https://api.telegram.org/bot{}/getMe",
        token.expose_secret()
    );
    match client.get(&url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
            if body.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                ProbeOutcome::ok("telegram: token hợp lệ (getMe ok)")
            } else {
                ProbeOutcome::error(format!(
                    "telegram: HTTP {status} nhưng getMe không ok (chi tiết đã redact)"
                ))
                .with_hint("kiểm lại token từ @BotFather")
            }
        }
        Err(error) => ProbeOutcome::error(format!(
            "telegram: không tới được api.telegram.org: {}",
            error.without_url()
        )),
    }
}

/// Request tối thiểu tới nhà cung cấp web search; key đã do người gọi resolve.
///
/// Client **no_proxy** + tắt redirect — giống hệt `SearchClient` của `bean-security`
/// để kết luận khớp hành vi thật của tool (K29).
pub async fn probe_search(
    provider: WebSearchProvider,
    base_url: Option<&str>,
    key: Option<&SecretString>,
) -> ProbeOutcome {
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("bean/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return ProbeOutcome::warn("web_search: không dựng được HTTP client");
    };
    let result = match provider {
        WebSearchProvider::Tavily => {
            let Some(key) = key else {
                return ProbeOutcome::error("web_search tavily: thiếu key");
            };
            client
                .post("https://api.tavily.com/search")
                .bearer_auth(key.expose_secret())
                .json(&json!({
                    "query": "bean doctor",
                    "search_depth": "basic",
                    "max_results": 1,
                    "include_answer": false,
                    "include_raw_content": false
                }))
                .send()
                .await
        }
        WebSearchProvider::Brave => {
            let Some(key) = key else {
                return ProbeOutcome::error("web_search brave: thiếu key");
            };
            let mut url = match url::Url::parse("https://api.search.brave.com/res/v1/web/search") {
                Ok(url) => url,
                Err(error) => {
                    return ProbeOutcome::warn(format!("web_search brave: URL hỏng: {error}"));
                }
            };
            url.query_pairs_mut()
                .append_pair("q", "bean doctor")
                .append_pair("count", "1");
            client
                .get(url)
                .header("X-Subscription-Token", key.expose_secret())
                .send()
                .await
        }
        WebSearchProvider::Searxng => {
            let Some(base) = base_url else {
                return ProbeOutcome::warn("web_search searxng: thiếu tools.web_search.base_url");
            };
            let Some(url) = searxng_search_url(base) else {
                return ProbeOutcome::warn("web_search searxng: base_url không phải http(s)");
            };
            client.get(url).send().await
        }
    };
    match result {
        Ok(response) => {
            let status = response.status().as_u16();
            match status {
                200..=299 => ProbeOutcome::ok(format!(
                    "web_search {} phản hồi (HTTP {status})",
                    search_provider_label(provider)
                )),
                401 | 403 => {
                    ProbeOutcome::error(format!("web_search: key bị từ chối (HTTP {status})"))
                        .with_hint("rotate key rồi `bean secret set`")
                }
                429 => ProbeOutcome::warn("web_search: HTTP 429 — quá giới hạn tần suất"),
                status => ProbeOutcome::warn(format!("web_search: HTTP {status}")),
            }
        }
        Err(error) => {
            ProbeOutcome::error(format!("web_search: lỗi request: {}", error.without_url()))
        }
    }
}

fn search_provider_label(provider: WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Tavily => "tavily",
        WebSearchProvider::Brave => "brave",
        WebSearchProvider::Searxng => "searxng",
    }
}

/// URL `/search` cho SearXNG tự host (kèm query `q`/`format`); `None` khi base
/// không phải http(s). Dựng query bằng `url::Url` như `bean-security` — reqwest 0.13
/// gate `.query()` sau feature riêng, workspace không bật.
#[must_use]
pub fn searxng_search_url(base: &str) -> Option<url::Url> {
    let trimmed = base.trim().trim_end_matches('/');
    let starts_ok = trimmed.starts_with("http://") || trimmed.starts_with("https://");
    if !starts_ok {
        return None;
    }
    let mut url = url::Url::parse(&format!("{trimmed}/search")).ok()?;
    url.query_pairs_mut()
        .append_pair("q", "bean doctor")
        .append_pair("format", "json");
    Some(url)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn searxng_url_building() {
        let url = searxng_search_url("http://localhost:8080/").unwrap();
        assert!(url.as_str().starts_with("http://localhost:8080/search?"));
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert!(pairs.contains(&("q".to_string(), "bean doctor".to_string())));
        assert!(pairs.contains(&("format".to_string(), "json".to_string())));

        let url = searxng_search_url(" https://s.example ").unwrap();
        assert!(url.as_str().starts_with("https://s.example/search?"));

        assert!(searxng_search_url("ftp://x").is_none());
    }

    #[test]
    fn probe_level_serializes_to_machine_strings() {
        assert_eq!(ProbeLevel::Ok.as_str(), "ok");
        assert_eq!(ProbeLevel::Warn.as_str(), "warn");
        assert_eq!(ProbeLevel::Error.as_str(), "error");
    }

    #[test]
    fn outcome_hints_are_additive() {
        let outcome = ProbeOutcome::error("thất bại").with_hint("thử lại");
        assert_eq!(outcome.level, ProbeLevel::Error);
        assert_eq!(outcome.hint.as_deref(), Some("thử lại"));
        assert_eq!(ProbeOutcome::ok("x").hint, None);
    }
}
