//! Dựng [`LlmProvider`] từ `[llm]` trong cấu hình (agents.md mục 5, 18).

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use bean_types::config::{LlmConfig, LlmProviderKind};

use crate::LlmError;
use crate::LlmProvider;
use crate::anthropic::AnthropicProvider;
use crate::openai_compat::OpenAiCompatProvider;

/// Dựng provider theo cấu hình.
///
/// `api_key` là secret đã resolve từ biến môi trường (tên biến được khai báo qua
/// `llm.api_key_env` — xem [`bean_types::config::Config::resolve_secrets`]).
///
/// * `anthropic`: **bắt buộc** có key.
/// * `openai_compat`: key tuỳ chọn khi `base_url` được đặt (Ollama/vLLM tự host);
///   thiếu key mà không có `base_url` là lỗi (mặc định trỏ tới OpenAI chính thức).
///
/// # Errors
/// [`LlmError::Config`] khi provider lạ, thiếu key, hoặc không dựng được HTTP client.
pub fn build_provider(
    config: &LlmConfig,
    api_key: Option<SecretString>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let key = api_key.filter(|k| !k.expose_secret().is_empty());
    match config.provider {
        LlmProviderKind::Anthropic => {
            let Some(key) = key else {
                return Err(LlmError::Config(format!(
                    "provider `anthropic` cần API key — đặt nó trong biến môi trường `{}` \
                     (khai báo bởi trường llm.api_key_env)",
                    config.api_key_env
                )));
            };
            let provider =
                AnthropicProvider::new(config.model.clone(), key, config.base_url.as_deref())?;
            Ok(Arc::new(provider))
        }
        LlmProviderKind::OpenAiCompat => {
            let provider =
                OpenAiCompatProvider::new(config.model.clone(), key, config.base_url.as_deref())?;
            Ok(Arc::new(provider))
        }
    }
}
