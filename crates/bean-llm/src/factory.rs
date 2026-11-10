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
/// # Vì sao `openai_compat` không kiểm tra key lúc dựng
/// Heuristic "có `base_url` thì key là tuỳ chọn" là cách duy nhất phân biệt server
/// tự host với server đám mây mà **không** thêm trường cấu hình mới — nhưng nó
/// không phân biệt được: Ollama cũng có `base_url` (`http://localhost:11434/v1`).
/// Nếu siết thành "luôn cần key", người dùng Ollama mất đường vào mà không có
/// cách nào tắt.
///
/// Nên thay vì đoán lúc khởi động, ta kiểm tra lúc **có bằng chứng**: request đi ra
/// mà không kèm `Authorization` và nhận 401/403 thì chắc chắn là thiếu key, và lúc
/// đó báo đúng tên biến cần đặt. Ngược lại (đã gửi key mà vẫn 401) thì giữ
/// nguyên lỗi của hãng, vì khi đó là key sai/hết hạn — bảo người dùng export lại
/// một key vốn đã có chỉ là dẫn họ đi sai.
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
                AnthropicProvider::new(config.model.clone(), key, config.base_url.as_deref())?
                    .with_prompt_cache(config.prompt_cache);
            Ok(Arc::new(provider))
        }
        LlmProviderKind::OpenAiCompat => {
            let provider = OpenAiCompatProvider::new(
                config.model.clone(),
                key,
                config.base_url.as_deref(),
                &config.api_key_env,
            )?;
            Ok(Arc::new(provider))
        }
    }
}

/// Dựng chuỗi provider dự phòng từ `[llm].fallback_profiles`.
///
/// Mỗi tên được resolve qua [`LlmConfig::profile`] (chấp nhận `"default"` = chính
/// `[llm]` ngầm định). Profile không dựng được (thiếu key…) thì **bỏ qua và log
/// cảnh báo** — dự phòng là tiện ích phòng khi sự cố, không được phép làm chết
/// khởi động vì lỗi cấu hình của chính nó. Provider chính do caller tự dựng và
/// gói bằng [`FailoverProvider::attach`].
#[must_use]
pub fn build_fallback_providers(
    config: &LlmConfig,
    resolve_profile_key: &dyn Fn(&bean_types::config::LlmProfile) -> Option<SecretString>,
) -> Vec<Arc<dyn LlmProvider>> {
    let mut out = Vec::new();
    for name in &config.fallback_profiles {
        let Some(profile) = config.profile(name) else {
            // validate.rs đã chặn tên lạ lúc khởi động; nếu vẫn tới đây (config bị
            // sửa tay runtime) thì bỏ qua thay vì chết.
            tracing::warn!(profile = %name, "fallback profile không tồn tại — bỏ qua");
            continue;
        };
        let mut candidate = config.clone();
        candidate.apply_profile(&profile);
        let key = resolve_profile_key(&profile);
        match build_provider(&candidate, key) {
            Ok(provider) => out.push(provider),
            Err(error) => {
                tracing::warn!(profile = %name, error = %error, "không dựng được fallback provider — bỏ qua");
            }
        }
    }
    out
}
