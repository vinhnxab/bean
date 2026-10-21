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
            let stored = Self::stored_api_key(profile);
            let key_configured = stored.is_some()
                || profile.api_key_env.trim().is_empty()
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
                api_key_hint: stored.map(|key| api_key_hint(&key)),
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

    /// Key dán trực tiếp trong profile (nếu có, sau trim); `None` = resolve từ env.
    fn stored_api_key(profile: &bean_types::config::LlmProfile) -> Option<String> {
        profile
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
    }

    /// Resolve API key của một profile: key dán trực tiếp trong `bean.toml` thắng
    /// biến môi trường; cả hai vắng thì `None` (provider không cần key).
    pub(crate) fn resolve_profile_api_key(
        profile: &bean_types::config::LlmProfile,
    ) -> Option<SecretString> {
        if let Some(key) = Self::stored_api_key(profile) {
            return Some(SecretString::from(key));
        }
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

    /// Chuẩn hoá `allowed_models` của profile trước khi lưu.
    ///
    /// Lọc phần tử rỗng (dán tham số tạo `""`/khoảng trắng) và khi danh sách không
    /// rỗng mà thiếu model mặc định thì chèn model vào đầu. Bắt buộc để config luôn
    /// nạp nổi: `validate.rs` từ chối khởi động `serve` khi model mặc định không
    /// nằm trong `allowed_models` của profile (sự cố thật 2026-10-20: phần tử dán
    /// thiếu dấu `,` khiến profile hợp lệ lúc lưu nhưng làm `serve` chết lúc load).
    fn normalize_allowed_models(profile: &mut bean_types::config::LlmProfile) {
        let mut models: Vec<String> = profile
            .allowed_models
            .iter()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .collect();
        if !models.is_empty() && models.iter().all(|model| *model != profile.model) {
            models.insert(0, profile.model.clone());
        }
        profile.allowed_models = models;
    }

    /// Chuẩn hoá `api_key` của profile trước khi lưu: rỗng/chỉ khoảng trắng ⇒ `None`.
    /// Trim ở đây (không để validate từ chối) vì copy-paste hay dính khoảng trắng.
    fn normalize_api_key(profile: &mut bean_types::config::LlmProfile) {
        profile.api_key = profile
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string);
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
        mut profile: bean_types::config::LlmProfile,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        Self::normalize_allowed_models(&mut profile);
        Self::normalize_api_key(&mut profile);
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
    /// `key_update` tách bạch semantics API key: [`ApiKeyUpdate::Keep`] giữ key đã
    /// lưu (form để trống), [`ApiKeyUpdate::Clear`] xoá (quay lại env),
    /// [`ApiKeyUpdate::Set`] ghi đè.
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
        key_update: ApiKeyUpdate,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        self.update_llm_provider_with(name, profile, key_update, &bean_llm::build_provider)
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
        key_update: ApiKeyUpdate,
        factory: &ProviderFactory<'_>,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        if name == bean_types::config::DEFAULT_PROFILE_NAME {
            return Err(RouterError::DefaultProfileReadonly);
        }
        profile.name = name.to_string();
        Self::normalize_allowed_models(&mut profile);

        // 1) Read lock: áp ApiKeyUpdate (Keep cần existing) rồi normalize + validate —
        //    key cuối cùng (sau Set/Clear/Keep) phải chịu toàn bộ ràng buộc.
        let config = read_lock(&self.inner.config)?;
        let existing_index = config
            .llm
            .providers
            .iter()
            .position(|existing| existing.name == name)
            .ok_or_else(|| RouterError::ProfileNotFound(name.to_string()))?;
        match &key_update {
            ApiKeyUpdate::Keep => {
                profile.api_key = config.llm.providers[existing_index].api_key.clone();
            }
            ApiKeyUpdate::Clear => profile.api_key = None,
            ApiKeyUpdate::Set(key) => profile.api_key = Some(key.clone()),
        }
        Self::normalize_api_key(&mut profile);
        profile
            .validate_for_storage()
            .map_err(|error| RouterError::ProfileInvalid(error.to_string()))?;
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
        let index = existing_index;
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

/// Gợi ý 4 ký tự cuối của key để người dùng nhận ra key nào đang lưu
/// (vd `"••••ed6"`). Đi qua `chars()` — cắt theo byte sẽ panic với UTF-8.
fn api_key_hint(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••{tail}")
}

#[cfg(test)]
mod tests {
    use super::api_key_hint;

    #[test]
    fn hint_shows_last_four_chars() {
        assert_eq!(api_key_hint("sk-abc123ed6"), "••••3ed6");
    }

    #[test]
    fn hint_never_panics_on_multibyte_input() {
        assert_eq!(api_key_hint("khóa-ữ"), "••••óa-ữ");
        assert_eq!(api_key_hint("ab"), "••••ab");
    }
}
