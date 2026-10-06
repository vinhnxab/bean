//! Giải secret từ biến môi trường (bọc `secrecy::SecretString`).

use secrecy::SecretString;

use super::*;

impl Config {
    /// Đọc secret từ biến môi trường của tiến trình hiện tại.
    ///
    /// Chính sách "fail fast có chừng mực" (D6.4):
    /// * `llm.api_key_env` — bắt buộc **chỉ khi** provider cần key (`anthropic` luôn cần;
    ///   `openai_compat` cần trừ khi `base_url` trỏ tới server tự host);
    /// * `tools.web_search.api_key_env` — **không** bắt buộc (chỉ cảnh báo khi nhóm `web`
    ///   bật mà thiếu; lỗi rõ ràng sẽ nổi tại lúc tool được gọi — M7);
    /// * `telegram.token_env` — bắt buộc khi `telegram.enabled`.
    ///
    /// # Errors
    /// Trả [`ConfigError::MissingEnv`] khi secret bắt buộc chưa được đặt (nêu rõ **tên biến**
    /// và **trường cấu hình** khai báo nó, không bao giờ nêu giá trị).
    pub fn resolve_secrets(&self) -> Result<ResolvedSecrets, ConfigError> {
        self.resolve_secrets_with(|name| std::env::var(name).ok())
    }
}
impl Config {
    /// Read only the Telegram bot token from its configured environment variable.
    ///
    /// This is separate from [`Config::resolve_secrets`] so `serve --fake-llm`
    /// can start Telegram without requiring an unused LLM API key.
    ///
    /// # Errors
    /// Returns [`ConfigError::MissingEnv`] when Telegram is enabled and its
    /// configured environment variable is absent or blank.
    pub fn resolve_telegram_token(&self) -> Result<Option<SecretString>, ConfigError> {
        if !self.telegram.enabled {
            return Ok(None);
        }
        let token = read_required(
            &|name: &str| std::env::var(name).ok(),
            "telegram.token_env",
            &self.telegram.token_env,
        )?;
        Ok(Some(token))
    }
}
impl Config {
    /// Đọc credential billing (M22a).
    ///
    /// Trả `None` khi billing tắt. Khi billing **bật** mà biến chưa được đặt thì trả lỗi
    /// rõ ràng — không âm thầm chạy stub, vì người vận hành cần biết ngay là chưa có key
    /// thay vì tưởng đã đọc được chi phí.
    ///
    /// # Errors
    /// [`ConfigError::MissingEnv`] kể billing bật mà biến `api_key_env` thiếu/rỗng.
    pub fn resolve_billing_key(&self) -> Result<Option<SecretString>, ConfigError> {
        if !self.billing.enabled {
            return Ok(None);
        }
        let key = read_required(
            &|name: &str| std::env::var(name).ok(),
            "billing.api_key_env",
            &self.billing.api_key_env,
        )?;
        Ok(Some(key))
    }
}
impl Config {
    /// Đọc credential publish của marketing (M24).
    ///
    /// Giống [`Self::resolve_billing_key`]: marketing bật mà thiếu biến thì **báo lỗi lúc
    /// khởi động**, không âm thầm chạy stub — vì `marketing_publish` là hành động không hoàn
    /// tác, người vận hành phải biết chắc là mình đã cấu hình.
    ///
    /// # Errors
    /// [`ConfigError::MissingEnv`] kể marketing bật mà biến `api_key_env` thiếu/rỗng.
    pub fn resolve_marketing_key(&self) -> Result<Option<SecretString>, ConfigError> {
        if !self.marketing.enabled {
            return Ok(None);
        }
        let key = read_required(
            &|name: &str| std::env::var(name).ok(),
            "marketing.api_key_env",
            &self.marketing.api_key_env,
        )?;
        Ok(Some(key))
    }
}
impl Config {
    /// Read optional API key for `web_search` from biến môi trường được cấu hình.
    ///
    /// Trả `None` khi nhóm `web` tắt, provider không cần key (SearXNG), hoặc biến chưa
    /// được đặt. Việc thiếu key không chặn khởi động; `web_search` sẽ báo rõ khi được gọi.
    #[must_use]
    pub fn resolve_web_search_api_key(&self) -> Option<SecretString> {
        self.resolve_web_search_api_key_with(&|name| std::env::var(name).ok())
    }
}
impl Config {
    fn resolve_web_search_api_key_with<P>(&self, get_env: &P) -> Option<SecretString>
    where
        P: Fn(&str) -> Option<String>,
    {
        let enabled = self.tools.enabled.iter().any(|group| group == "web");
        if enabled && self.tools.web_search.provider.requires_api_key() {
            read_optional(get_env, &self.tools.web_search.api_key_env)
        } else {
            None
        }
    }
}
impl Config {
    /// Như [`Config::resolve_secrets`] nhưng nhận hàm đọc biến môi trường tuỳ ý.
    ///
    /// Dùng cho test để không phải `set_var` (trong Rust 2024 `set_var` là `unsafe` và sẽ
    /// phá vỡ `#![forbid(unsafe_code)]` của workspace).
    ///
    /// # Errors
    /// Như [`Config::resolve_secrets`].
    pub fn resolve_secrets_with<P>(&self, get_env: P) -> Result<ResolvedSecrets, ConfigError>
    where
        P: Fn(&str) -> Option<String>,
    {
        let llm_key = read_optional(&get_env, &self.llm.api_key_env);
        // Fail-fast chỉ khi provider thực sự cần key:
        // * `anthropic` — luôn cần;
        // * `openai_compat` — cần trừ khi `base_url` trỏ tới server tự host (Ollama/vLLM).
        // Lỗi khác "thiếu key" (ví dụ endpoint từ chối 401) sẽ nổi lên khi gọi API.
        let llm_key_needed = match self.llm.provider {
            LlmProviderKind::Anthropic => true,
            LlmProviderKind::OpenAiCompat => self.llm.base_url.is_none(),
        };
        if llm_key_needed && llm_key.is_none() {
            return Err(ConfigError::MissingEnv {
                field: "llm.api_key_env",
                env: self.llm.api_key_env.clone(),
            });
        }

        // Key web_search **không** bắt buộc lúc khởi động: `chat` phải dùng được mà không
        // cần Tavily (D6.4). Khi nhóm `web` bật mà thiếu key thì chỉ cảnh báo; M7 sẽ trả
        // lỗi rõ ràng ngay tại lúc tool `web_search` được gọi mà không có key.
        let web_tools_enabled = self.tools.enabled.iter().any(|group| group == "web");
        let web_search_api_key = self.resolve_web_search_api_key_with(&get_env);
        if web_tools_enabled
            && self.tools.web_search.provider.requires_api_key()
            && web_search_api_key.is_none()
        {
            tracing::warn!(
                env = %self.tools.web_search.api_key_env,
                "nhóm tool `web` đang bật nhưng chưa đặt biến môi trường cho key tìm kiếm — tool `web_search` sẽ báo lỗi khi được gọi"
            );
        }

        let telegram_token = if self.telegram.enabled {
            Some(read_required(
                &get_env,
                "telegram.token_env",
                &self.telegram.token_env,
            )?)
        } else {
            None
        };

        Ok(ResolvedSecrets {
            llm_api_key: llm_key,
            web_search_api_key,
            telegram_token,
        })
    }
}
/// Đọc env thành secret; trả `None` khi biến chưa đặt hoặc giá trị chỉ toàn khoảng trắng.
pub(super) fn read_optional<P>(get_env: &P, env: &str) -> Option<SecretString>
where
    P: Fn(&str) -> Option<String>,
{
    get_env(env)
        .filter(|value| !value.trim().is_empty())
        .map(SecretString::from)
}

/// Đọc env thành secret; lỗi [`ConfigError::MissingEnv`] khi thiếu (nêu tên biến + trường).
pub(super) fn read_required<P>(
    get_env: &P,
    field: &'static str,
    env: &str,
) -> Result<SecretString, ConfigError>
where
    P: Fn(&str) -> Option<String>,
{
    read_optional(get_env, env).ok_or_else(|| ConfigError::MissingEnv {
        field,
        env: env.to_string(),
    })
}
