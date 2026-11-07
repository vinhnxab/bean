//! Registry "secret slot" — danh sách tên secret hợp lệ, dẫn xuất từ cấu hình (M29
//! mục 4.1).
//!
//! Slot là **bộ cửa duy nhất** cho phép ghi vào `secrets.env`: người dùng chỉ đặt được
//! giá trị cho tên biến mà `bean.toml` đang khai báo qua các trường `*_env`. Nhờ vậy
//! file không thể trở thành đường chèn biến môi trường tuỳ ý. Slot thay đổi khi cấu
//! hình đổi (thêm profile LLM, đổi `api_key_env`…) — registry luôn dựng lại từ
//! [`bean_types::config::Config`], không hardcode tên ở chỗ khác.

use bean_types::config::Config;

/// Nhóm slot — dùng để hiển thị theo mục trong UI/CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotGroup {
    /// LLM và các profile `[[llm.providers]]`.
    Llm,
    /// Giọng nói (ElevenLabs).
    Voice,
    /// Bot Telegram.
    Telegram,
    /// Tìm kiếm web (Tavily/Brave/SearXNG).
    Search,
    /// Đọc chi phí cloud (M22a).
    Billing,
    /// Đăng bài marketing (M24).
    Marketing,
}

impl SlotGroup {
    /// Nhãn hiển thị cho người dùng (màn Cài đặt chia mục theo nhóm này).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Llm => "Mô hình & provider",
            Self::Voice => "Giọng nói",
            Self::Telegram => "Telegram",
            Self::Search => "Tìm kiếm web",
            Self::Billing => "Billing",
            Self::Marketing => "Marketing",
        }
    }
}

/// Một secret đã khai báo: tên biến, nhóm và nơi sử dụng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSlot {
    /// Tên biến môi trường (`[A-Z_][A-Z0-9_]*`) — cũng là tên dòng trong
    /// `secrets.env`.
    pub name: String,
    /// Nhóm hiển thị.
    pub group: SlotGroup,
    /// Các nơi dùng (một tên có thể được nhiều profile LLM chia sẻ).
    pub used_by: Vec<String>,
}

/// Tập slot hợp lệ tại thời điểm hiện tại của cấu hình.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretSlotRegistry {
    slots: Vec<SecretSlot>,
}

impl SecretSlotRegistry {
    /// Dẫn xuất slot từ cấu hình: mọi trường `*_env` của llm (mặc định + từng
    /// profile), web search, Telegram, billing, marketing và voice. Tên rỗng (provider
    /// tự host không cần key) hoặc không đúng dạng tên biến môi trường bị bỏ qua.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let mut slots: Vec<SecretSlot> = Vec::new();
        let mut push = |name: &str, group: SlotGroup, used_by: String| {
            let name = name.trim();
            if !is_env_name(name) {
                return;
            }
            if let Some(existing) = slots.iter_mut().find(|slot| slot.name == name) {
                if !existing.used_by.iter().any(|used| used == &used_by) {
                    existing.used_by.push(used_by);
                }
                return;
            }
            slots.push(SecretSlot {
                name: name.to_string(),
                group,
                used_by: vec![used_by],
            });
        };

        push(
            &config.llm.api_key_env,
            SlotGroup::Llm,
            format!("llm: {}", config.llm.active_profile_name()),
        );
        for profile in &config.llm.providers {
            push(
                &profile.api_key_env,
                SlotGroup::Llm,
                format!("llm: {}", profile.name),
            );
        }
        push(
            &config.tools.web_search.api_key_env,
            SlotGroup::Search,
            "web_search".to_string(),
        );
        push(
            &config.telegram.token_env,
            SlotGroup::Telegram,
            "telegram".to_string(),
        );
        push(
            &config.billing.api_key_env,
            SlotGroup::Billing,
            "billing".to_string(),
        );
        push(
            &config.marketing.api_key_env,
            SlotGroup::Marketing,
            "marketing".to_string(),
        );
        push(
            &config.voice.api_key_env,
            SlotGroup::Voice,
            "voice".to_string(),
        );

        slots.sort_by(|left, right| left.name.cmp(&right.name));
        Self { slots }
    }

    /// Slot theo tên; `None` khi tên không được khai báo ở đâu cả.
    #[must_use]
    pub fn slot(&self, name: &str) -> Option<&SecretSlot> {
        self.slots.iter().find(|slot| slot.name == name)
    }

    /// Toàn bộ slot, đã sắp theo tên.
    #[must_use]
    pub fn slots(&self) -> &[SecretSlot] {
        &self.slots
    }

    /// `true` khi cấu hình không khai báo secret nào.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Dựng registry từ danh sách tên tường minh (tên sai dạng bị bỏ qua).
    ///
    /// Hai nơi dùng: (1) test — không phải dựng cả `Config`; (2) M29 mục 4.1 —
    /// ghi key cho profile **mới** từ UI, tên biến được sinh ra
    /// (`LLM_<TÊN>_API_KEY`) chưa có trong `bean.toml` nên không dẫn xuất được
    /// từ [`Self::from_config`]. Registry tạm này chỉ phục vụ một lần `set`.
    #[must_use]
    pub fn from_names(names: &[&str]) -> Self {
        let mut slots: Vec<SecretSlot> = names
            .iter()
            .filter(|name| is_env_name(name))
            .map(|name| SecretSlot {
                name: (*name).to_string(),
                group: SlotGroup::Llm,
                used_by: vec!["test".to_string()],
            })
            .collect();
        slots.sort_by(|left, right| left.name.cmp(&right.name));
        Self { slots }
    }
}

/// Tên biến môi trường hợp lệ: `[A-Z_][A-Z0-9_]*`.
#[must_use]
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_');
    first_ok && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use bean_types::config::{LlmProfile, LlmProviderKind, default_max_tokens};

    fn profile(name: &str, api_key_env: &str) -> LlmProfile {
        LlmProfile {
            name: name.to_string(),
            provider: LlmProviderKind::OpenAiCompat,
            model: "test-model".to_string(),
            allowed_models: Vec::new(),
            api_key_env: api_key_env.to_string(),
            api_key: None,
            base_url: None,
            max_tokens: default_max_tokens(),
        }
    }

    #[test]
    fn default_config_yields_expected_groups() {
        let registry = SecretSlotRegistry::from_config(&Config::default());
        let find = |name: &str| {
            registry
                .slot(name)
                .map(|slot| (slot.group, slot.used_by.clone()))
        };
        // Mặc định của bean-types: ANTHROPIC_API_KEY (llm), TAVILY_API_KEY (search),
        // TELEGRAM_BOT_TOKEN (telegram), ELEVENLABS_KEY (voice)…
        assert_eq!(
            find("ANTHROPIC_API_KEY").map(|(group, _)| group),
            Some(SlotGroup::Llm)
        );
        assert_eq!(
            find("TAVILY_API_KEY").map(|(group, _)| group),
            Some(SlotGroup::Search)
        );
        assert_eq!(
            find("TELEGRAM_BOT_TOKEN").map(|(group, _)| group),
            Some(SlotGroup::Telegram)
        );
        assert_eq!(
            find("ELEVENLABS_KEY").map(|(group, _)| group),
            Some(SlotGroup::Voice)
        );
        // Slot sắp theo tên, không trùng.
        let names: Vec<&str> = registry
            .slots()
            .iter()
            .map(|slot| slot.name.as_str())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn shared_env_name_aggregates_used_by() {
        let mut config = Config::default();
        config.llm.api_key_env = "OPENROUTER_API_KEY".to_string();
        config.llm.providers = vec![
            profile("openrouter", "OPENROUTER_API_KEY"),
            profile("openrouter-2", "OPENROUTER_API_KEY"),
            profile("ollama", ""),
        ];
        let registry = SecretSlotRegistry::from_config(&config);
        let slot = registry.slot("OPENROUTER_API_KEY").unwrap();
        assert_eq!(slot.group, SlotGroup::Llm);
        assert_eq!(slot.used_by.len(), 3, "default + 2 profile dùng chung tên");
        assert!(
            registry.slot("OLLAMA_MISSING").is_none(),
            "env rỗng ⇒ không slot"
        );
    }

    #[test]
    fn invalid_env_names_are_skipped() {
        let mut config = Config::default();
        config.llm.api_key_env = "not-valid".to_string();
        config.telegram.token_env = "1BAD".to_string();
        let registry = SecretSlotRegistry::from_config(&config);
        assert!(registry.slot("not-valid").is_none());
        assert!(registry.slot("1BAD").is_none());
    }

    #[test]
    fn is_env_name_follows_shell_rules() {
        assert!(is_env_name("ANTHROPIC_API_KEY"));
        assert!(is_env_name("_PRIVATE"));
        assert!(!is_env_name("1BAD"));
        assert!(!is_env_name("A-B"));
        assert!(!is_env_name(""));
        assert!(!is_env_name("path"));
    }
}
