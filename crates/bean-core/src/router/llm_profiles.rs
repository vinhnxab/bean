//! CRUD `[[llm.providers]]` từ UI (Giai đoạn 3): thêm/sửa/xoá profile với persist
//! `bean.toml` giữ định dạng bằng `toml_edit`, và các ràng buộc transactional.

use super::*;
use secrecy::SecretString;

use super::status::ProviderFactory;

impl Router {
    /// Danh sách profile có thể switch, kèm trạng thái key + active — đã khử secret.
    ///
    /// # Errors
    /// [`RouterError::StatePoisoned`] nếu không đọc được cấu hình.
    pub fn llm_profiles(&self) -> Result<Vec<LlmProfileInfo>, RouterError> {
        let config = read_lock(&self.inner.config)?;
        let active = config.llm.active_profile_name().to_string();
        let mut out = Vec::new();
        // `default` là snapshot gốc, không phải `[llm]` hiện tại (đã có thể bị apply).
        let mut push = |profile: &bean_types::config::LlmProfile| {
            let key_configured = profile.api_key_env.trim().is_empty()
                || std::env::var(&profile.api_key_env).is_ok_and(|value| !value.trim().is_empty());
            out.push(LlmProfileInfo {
                name: profile.name.clone(),
                provider: profile.provider,
                model: profile.model.clone(),
                allowed_models: profile.effective_allowed_models(),
                base_url: profile.base_url.clone(),
                max_tokens: profile.max_tokens,
                api_key_env: profile.api_key_env.clone(),
                key_configured,
                active: profile.name == active,
            });
        };
        push(&self.inner.default_profile);
        for profile in &config.llm.providers {
            if profile.name != bean_types::config::DEFAULT_PROFILE_NAME {
                push(profile);
            }
        }
        Ok(out)
    }

    /// CRUD profile từ UI khả dụng hay không (cần `config_path` trỏ tới `bean.toml`).
    ///
    /// `false` (`--fake-llm`, `chat`, test) ⇒ adapter web phải từ chối mutation
    /// với `config_readonly` thay vì cho sửa một thứ sẽ mất khi restart.
    #[must_use]
    pub fn llm_profiles_writable(&self) -> bool {
        self.inner.config_path.is_some()
    }

    /// Resolve API key của một profile từ biến môi trường; rỗng ⇒ `None`
    /// (provider không cần key).
    fn resolve_profile_api_key(profile: &bean_types::config::LlmProfile) -> Option<SecretString> {
        if profile.api_key_env.trim().is_empty() {
            None
        } else {
            std::env::var(&profile.api_key_env)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(SecretString::from)
        }
    }

    /// Ghi danh sách `[[llm.providers]]` ra `bean.toml` (nếu Router được cấp đường dẫn).
    ///
    /// Gọi khi đang giữ lock config; tham số là hình thái **mới** của danh sách.
    /// Lỗi đĩa trả [`RouterError::ConfigPersist`] để caller từ chối nguyên thay đổi
    /// (transactional như [`Router::set_llm_model`]).
    fn persist_llm_providers(
        &self,
        providers: &[bean_types::config::LlmProfile],
    ) -> Result<(), RouterError> {
        let Some(path) = &self.inner.config_path else {
            return Ok(());
        };
        bean_types::config::write_llm_providers(path, providers)
            .map_err(|error| RouterError::ConfigPersist(error.to_string()))
    }

    /// Thêm profile vào `[[llm.providers]]` — **không** switch runtime.
    ///
    /// Thứ tự transactional: validate → persist `bean.toml` → mutate RAM. Lỗi bất
    /// kỳ ⇒ config giữ nguyên.
    ///
    /// # Errors
    /// * [`RouterError::ProfileInvalid`] — tên/trường không đạt ràng buộc.
    /// * [`RouterError::ProfileExists`] — đã có profile cùng tên.
    /// * [`RouterError::ConfigPersist`] — không ghi được `bean.toml`.
    /// * [`RouterError::StatePoisoned`] — lock bị poison.
    pub fn add_llm_provider(
        &self,
        profile: bean_types::config::LlmProfile,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        profile
            .validate_for_storage()
            .map_err(|error| RouterError::ProfileInvalid(error.to_string()))?;
        let mut config = write_lock(&self.inner.config)?;
        if config
            .llm
            .providers
            .iter()
            .any(|existing| existing.name == profile.name)
        {
            return Err(RouterError::ProfileExists(profile.name.clone()));
        }
        bean_types::config::validate_profile_name(&profile.name, &config.llm.providers)
            .map_err(|error| RouterError::ProfileInvalid(error.to_string()))?;
        let mut new_list = config.llm.providers.clone();
        new_list.push(profile.clone());
        self.persist_llm_providers(&new_list)?;
        config.llm.providers.push(profile);
        drop(config);
        self.llm_config()
    }

    /// Sửa profile trong `[[llm.providers]]`; tên lấy từ đường dẫn API, body không
    /// có quyền đổi tên (đổi tên = tạo profile mới + xoá cũ).
    ///
    /// Nếu profile đang active: dựng provider với trường mới **trước khi** giữ write
    /// lock (bước fail-able), rồi persist → apply — runtime chuyển sang hình thái
    /// mới ngay. Lỗi dựng provider (thiếu key…) ⇒ nguyên trạng cũ.
    ///
    /// # Errors
    /// * [`RouterError::DefaultProfileReadonly`] — `name == "default"`.
    /// * [`RouterError::ProfileNotFound`] — profile không tồn tại.
    /// * [`RouterError::ProfileInvalid`] — trường không đạt ràng buộc.
    /// * [`RouterError::LlmProvider`] — profile active và factory dựng thất bại.
    /// * [`RouterError::ConfigPersist`] / [`RouterError::Persist`] — lỗi đĩa.
    /// * [`RouterError::StatePoisoned`] — lock bị poison.
    pub fn update_llm_provider(
        &self,
        name: &str,
        profile: bean_types::config::LlmProfile,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        self.update_llm_provider_with(name, profile, &bean_llm::build_provider)
    }

    /// Như [`Router::update_llm_provider`] nhưng nhận factory — test dựng provider
    /// giả không cần mạng.
    ///
    /// # Errors
    /// Như [`Router::update_llm_provider`].
    pub fn update_llm_provider_with(
        &self,
        name: &str,
        mut profile: bean_types::config::LlmProfile,
        factory: &ProviderFactory<'_>,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        if name == bean_types::config::DEFAULT_PROFILE_NAME {
            return Err(RouterError::DefaultProfileReadonly);
        }
        profile.name = name.to_string();
        profile
            .validate_for_storage()
            .map_err(|error| RouterError::ProfileInvalid(error.to_string()))?;

        // 1) Read lock: resolve + dựng candidate nếu profile đang active (fail-able).
        let config = read_lock(&self.inner.config)?;
        if !config
            .llm
            .providers
            .iter()
            .any(|existing| existing.name == name)
        {
            return Err(RouterError::ProfileNotFound(name.to_string()));
        }
        let active = config.llm.active_profile_name() == name;
        let provider = if active {
            let api_key = Self::resolve_profile_api_key(&profile);
            let mut candidate = config.llm.clone();
            candidate.apply_profile(&profile);
            Some(
                factory(&candidate, api_key)
                    .map_err(|error| RouterError::LlmProvider(error.to_string()))?,
            )
        } else {
            None
        };
        drop(config);

        // 2) Write lock: persist toml với danh sách mới TRƯỚC khi mutate RAM.
        let mut config = write_lock(&self.inner.config)?;
        let index = config
            .llm
            .providers
            .iter()
            .position(|existing| existing.name == name)
            .ok_or_else(|| RouterError::ProfileNotFound(name.to_string()))?;
        let mut new_list = config.llm.providers.clone();
        new_list[index] = profile.clone();
        self.persist_llm_providers(&new_list)?;
        config.llm.providers[index] = profile;
        if active {
            let applied = config.llm.providers[index].clone();
            config.llm.apply_profile(&applied);
            self.persist_llm_state(name, &config.llm.model)?;
            if let Some(provider) = provider {
                let mut current = write_lock(&self.inner.llm)?;
                *current = provider;
            }
        }
        drop(config);
        self.llm_config()
    }

    /// Xoá profile khỏi `[[llm.providers]]`.
    ///
    /// Chặn xoá profile đang active ([`RouterError::ProfileActive`]) — tránh trạng
    /// thái runtime mồ côi (provider đang chạy nhưng định nghĩa đã mất). UI gợi ý
    /// switch sang profile khác trước.
    ///
    /// # Errors
    /// * [`RouterError::DefaultProfileReadonly`] — `name == "default"`.
    /// * [`RouterError::ProfileNotFound`] — profile không tồn tại.
    /// * [`RouterError::ProfileActive`] — profile đang có hiệu lực.
    /// * [`RouterError::ConfigPersist`] — không ghi được `bean.toml`.
    /// * [`RouterError::StatePoisoned`] — lock bị poison.
    pub fn delete_llm_provider(&self, name: &str) -> Result<LlmRuntimeConfig, RouterError> {
        if name == bean_types::config::DEFAULT_PROFILE_NAME {
            return Err(RouterError::DefaultProfileReadonly);
        }
        let mut config = write_lock(&self.inner.config)?;
        let index = config
            .llm
            .providers
            .iter()
            .position(|existing| existing.name == name)
            .ok_or_else(|| RouterError::ProfileNotFound(name.to_string()))?;
        if config.llm.active_profile_name() == name {
            return Err(RouterError::ProfileActive(name.to_string()));
        }
        let mut new_list = config.llm.providers.clone();
        new_list.remove(index);
        self.persist_llm_providers(&new_list)?;
        config.llm.providers.remove(index);
        drop(config);
        self.llm_config()
    }
}
