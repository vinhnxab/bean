//! Section `[llm]`, `[tools.web_search]`, `[learning]`.

use serde::{Deserialize, Serialize};

use super::*;

/// `[llm]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlmConfig {
    /// Provider đang dùng.
    pub provider: LlmProviderKind,
    /// Model đang dùng.
    pub model: String,
    /// Danh sách model được phép cho `/model` (mục 10). Rỗng ⇒ chỉ `model`.
    pub allowed_models: Vec<String>,
    /// Tên biến môi trường chứa API key (**không** phải key).
    pub api_key_env: String,
    /// Ghi đè base URL (dùng cho endpoint không phải OpenAI chính thức / Ollama).
    pub base_url: Option<String>,
    /// Trần token cho mỗi lượt gọi.
    pub max_tokens: u32,
    /// Các profile khai báo tường minh qua `[[llm.providers]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<LlmProfile>,
    /// Profile đang hoạt động — trạng thái runtime, **không** nằm trong `bean.toml`.
    ///
    /// Được persist riêng ở `data.dir/llm-state.json` (module [`llm_state`]). Rỗng
    /// (mặc định serde `skip`) coi như [`DEFAULT_PROFILE_NAME`].
    #[serde(skip)]
    pub active_profile: String,
}

/// Tên profile ngầm định suy từ chính `[llm]` (không cần khai báo tường minh).
pub const DEFAULT_PROFILE_NAME: &str = "default";

fn default_max_tokens() -> u32 {
    4096
}

/// Một cấu hình provider hoàn chỉnh, khai báo qua `[[llm.providers]]`.
///
/// Profile là "bản sao" của `[llm]` mang tên: switch profile = copy các trường vào
/// `[llm]` và dựng lại provider runtime (hot-swap). `name = "default"` được dành
/// riêng cho profile ngầm định, không dùng làm tên tường minh.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LlmProfile {
    /// Tên profile, tham chiếu khi switch (ví dụ `ollama`, `openrouter`).
    pub name: String,
    /// Provider LLM.
    pub provider: LlmProviderKind,
    /// Model mặc định khi switch sang profile này.
    pub model: String,
    /// Danh sách model được phép chuyển runtime trong profile. Rỗng ⇒ chỉ `model`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_models: Vec<String>,
    /// Tên biến môi trường chứa API key; rỗng ⇒ provider không cần key (Ollama/vLLM).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key_env: String,
    /// Base URL ghi đè (endpoint tự host / gateway).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Trần token cho mỗi lượt gọi.
    #[serde(
        default = "default_max_tokens",
        skip_serializing_if = "is_default_max_tokens"
    )]
    pub max_tokens: u32,
}

fn is_default_max_tokens(value: &u32) -> bool {
    *value == default_max_tokens()
}

impl LlmProfile {
    /// Danh sách model mà switch runtime được phép chọn trong profile này.
    #[must_use]
    pub fn effective_allowed_models(&self) -> Vec<String> {
        if self.allowed_models.is_empty() {
            vec![self.model.clone()]
        } else {
            self.allowed_models.clone()
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProviderKind::Anthropic,
            model: "claude-sonnet-5".to_string(),
            allowed_models: Vec::new(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: None,
            max_tokens: 4096,
            providers: Vec::new(),
            active_profile: DEFAULT_PROFILE_NAME.to_string(),
        }
    }
}

impl LlmConfig {
    /// Danh sách model mà `/model` được phép chuyển sang.
    #[must_use]
    pub fn effective_allowed_models(&self) -> Vec<String> {
        if self.allowed_models.is_empty() {
            vec![self.model.clone()]
        } else {
            self.allowed_models.clone()
        }
    }

    /// Tên profile đang hoạt động, đã chuẩn hoá (rỗng ⇒ `"default"`).
    #[must_use]
    pub fn active_profile_name(&self) -> &str {
        if self.active_profile.is_empty() {
            DEFAULT_PROFILE_NAME
        } else {
            &self.active_profile
        }
    }

    /// Tất cả profile có thể switch: profile ngầm định `"default"` đứng đầu,
    /// theo sau là các `[[llm.providers]]` theo thứ tự khai báo.
    #[must_use]
    pub fn profiles(&self) -> Vec<LlmProfile> {
        let mut out = vec![self.as_profile()];
        out.extend(
            self.providers
                .iter()
                .filter(|profile| profile.name != DEFAULT_PROFILE_NAME)
                .cloned(),
        );
        out
    }

    /// Profile theo tên; `"default"` trả profile ngầm định suy từ chính `[llm]`.
    #[must_use]
    pub fn profile(&self, name: &str) -> Option<LlmProfile> {
        if name == DEFAULT_PROFILE_NAME {
            return Some(self.as_profile());
        }
        self.providers.iter().find(|p| p.name == name).cloned()
    }

    /// Suy `[llm]` hiện tại thành profile ngầm định `"default"`.
    #[must_use]
    pub fn as_profile(&self) -> LlmProfile {
        LlmProfile {
            name: DEFAULT_PROFILE_NAME.to_string(),
            provider: self.provider,
            model: self.model.clone(),
            allowed_models: self.allowed_models.clone(),
            api_key_env: self.api_key_env.clone(),
            base_url: self.base_url.clone(),
            max_tokens: self.max_tokens,
        }
    }

    /// Copy các trường của profile vào `[llm]` (giữ nguyên `providers`) và đánh dấu
    /// `active_profile`. Đây là **duy nhất** nơi ghi profile vào config runtime.
    pub fn apply_profile(&mut self, profile: &LlmProfile) {
        self.provider = profile.provider;
        self.model = profile.model.clone();
        self.allowed_models = profile.allowed_models.clone();
        self.api_key_env = profile.api_key_env.clone();
        self.base_url = profile.base_url.clone();
        self.max_tokens = profile.max_tokens;
        self.active_profile = profile.name.clone();
    }
}

/// `[tools.web_search]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebSearchConfig {
    /// Provider tìm kiếm.
    pub provider: WebSearchProvider,
    /// Tên biến môi trường chứa API key (không dùng khi `provider = "searxng"`).
    pub api_key_env: String,
    /// Base URL của SearXNG tự host.
    pub base_url: Option<String>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: WebSearchProvider::Tavily,
            api_key_env: "TAVILY_API_KEY".to_string(),
            base_url: None,
        }
    }
}

/// Cấu hình learning loop sau run thành công (agents.md mục 17, milestone M15).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LearningConfig {
    /// Bật reflection và tạo đề xuất skill.
    pub enabled: bool,
    /// Số tool call tối thiểu trong một run trước khi reflection.
    pub min_tool_calls: u32,
    /// Khoảng cách tối thiểu giữa hai đề xuất, tính bằng phút.
    pub proposal_interval_minutes: u64,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_tool_calls: 5,
            proposal_interval_minutes: 60,
        }
    }
}
