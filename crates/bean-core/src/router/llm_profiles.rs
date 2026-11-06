//! CRUD `[[llm.providers]]` từ UI (Giai đoạn 3): thêm/sửa/xoá profile với persist
//! `bean.toml` giữ định dạng bằng `toml_edit`, và các ràng buộc transactional.

use super::*;
use secrecy::{ExposeSecret, SecretString};

use super::status::ProviderFactory;

impl Router {
    /// Snapshot secret hiện tại của tiến trình; `None` khi Router không được cấp
    /// khoá (test, `chat`, `mcp`) — resolve khi đó chỉ còn biến môi trường.
    /// File đọc lỗi chỉ warn: resolve vẫn chạy với env, không làm run chết.
    pub fn secret_snapshot(&self) -> Option<bean_settings::SecretSnapshot> {
        let store = self.inner.options.secrets.as_ref()?;
        match store.snapshot() {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "không đọc được secrets.env — key chỉ resolve từ biến môi trường"
                );
                None
            }
        }
    }

    /// Kho secret của tiến trình; `None` khi không được cấp (test/`chat`).
    pub fn secret_store(&self) -> Option<&Arc<bean_settings::SecretStore>> {
        self.inner.options.secrets.as_ref()
    }

    /// Resolve API key của một profile theo thứ tự D19.3: env > `secrets.env` >
    /// chưa đặt. Trường `api_key` trong profile (legacy trước M29) **bị bỏ qua**.
    pub(crate) fn resolve_profile_api_key(
        &self,
        profile: &bean_types::config::LlmProfile,
    ) -> Option<SecretString> {
        let snapshot = self.secret_snapshot();
        let get_env = bean_settings::env_first_lookup(snapshot.as_ref());
        Self::resolve_key_with(profile, &get_env)
    }

    /// Lookup key của profile qua hàm đọc env tuỳ ý; tên rỗng = provider tự host,
    /// không cần key.
    fn resolve_key_with(
        profile: &bean_types::config::LlmProfile,
        get_env: &dyn Fn(&str) -> Option<String>,
    ) -> Option<SecretString> {
        if profile.api_key_env.trim().is_empty() {
            return None;
        }
        get_env(&profile.api_key_env).map(SecretString::from)
    }

    /// Danh sách profile có thể switch, kèm trạng thái key + active — đã khử secret.
    ///
    /// # Errors
    /// [`RouterError::StatePoisoned`] nếu không đọc được cấu hình.
    pub fn llm_profiles(&self) -> Result<Vec<LlmProfileInfo>, RouterError> {
        let config = read_lock(&self.inner.config)?;
        let active = config.llm.active_profile_name().to_string();
        let snapshot = self.secret_snapshot();
        let get_env = bean_settings::env_first_lookup(snapshot.as_ref());
        let mut out = Vec::new();
        // `default` là snapshot gốc, không phải `[llm]` hiện tại (đã có thể bị apply).
        let mut push = |profile: &bean_types::config::LlmProfile| {
            let resolved = Self::resolve_key_with(profile, &get_env);
            let key_configured = profile.api_key_env.trim().is_empty() || resolved.is_some();
            let api_key_hint = resolved
                .as_ref()
                .map(|key| api_key_hint(key.expose_secret()));
            out.push(LlmProfileInfo {
                name: profile.name.clone(),
                provider: profile.provider,
                model: profile.model.clone(),
                allowed_models: profile.effective_allowed_models(),
                base_url: profile.base_url.clone(),
                max_tokens: profile.max_tokens,
                api_key_env: profile.api_key_env.clone(),
                key_configured,
                api_key_hint,
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

    /// Đường dẫn `bean.toml` mà Router biết — ngoài persist CRUD profile, đây cũng là
    /// nguồn ghi của REST `/api/settings` (M29): `None` (`chat`, `--fake-llm`, test)
    /// ⇒ adapter web trả `config_readonly` thay vì ghi một thứ sẽ mất khi restart.
    #[must_use]
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.inner.config_path.as_deref()
    }

    /// Key dán trực tiếp trong profile (nếu có, sau trim); `None` = resolve từ env.
    ///
    /// Deprecated M29: chỉ còn dùng để **di chuyển** key legacy sang `secrets.env`
    /// (xem [`Router::absorb_profile_api_key`]); resolve không đọc trường này nữa.
    fn legacy_stored_api_key(profile: &bean_types::config::LlmProfile) -> Option<String> {
        profile
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
    }

    /// Ghi key dán trong `profile.api_key` vào `secrets.env` (M29 mục 4.1) rồi xoá
    /// trường đó đi: key **không bao giờ** được chạm `bean.toml`.
    ///
    /// Sinh `api_key_env` khi profile chưa có (rỗng): `LLM_<TÊN>_API_KEY`. Không có
    /// khoá secret (test/`chat`) ⇒ [`RouterError::SecretUnavailable`].
    ///
    /// # Errors
    /// [`RouterError::SecretUnavailable`] khi thiếu khoá, hoặc lỗi đĩa khi ghi.
    fn absorb_profile_api_key(
        &self,
        profile: &mut bean_types::config::LlmProfile,
    ) -> Result<(), RouterError> {
        let pasted = profile
            .api_key
            .take()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        let Some(key) = pasted else {
            return Ok(());
        };
        if profile.api_key_env.trim().is_empty() {
            profile.api_key_env = Self::derive_env_name(&profile.name);
        }
        self.write_profile_secret(&profile.api_key_env, &key)
    }

    /// Tên biến sinh cho profile mới: `LLM_<TÊN>_API_KEY`, ký tự ngoài `[A-Z0-9]`
    /// thay bằng `_`. Luôn hợp lệ (`is_env_name`) vì bắt đầu bằng `LLM_`.
    fn derive_env_name(name: &str) -> String {
        let sanitized: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
            .to_ascii_uppercase();
        format!("LLM_{sanitized}_API_KEY")
    }

    /// Ghi một giá trị vào `secrets.env` dưới slot `env_name` (0600, atomic).
    ///
    /// # Errors
    /// [`RouterError::SecretUnavailable`] khi thiếu khoá; lỗi đĩa bọc cùng kiểu.
    pub(crate) fn write_profile_secret(
        &self,
        env_name: &str,
        value: &str,
    ) -> Result<(), RouterError> {
        let Some(store) = self.secret_store() else {
            return Err(RouterError::SecretUnavailable);
        };
        let registry =
            bean_settings::SecretSlotRegistry::from_names(std::slice::from_ref(&env_name));
        store
            .set(&registry, env_name, SecretString::from(value.to_string()))
            .map_err(|error| RouterError::SecretWrite(error.to_string()))
    }

    /// Xoá một slot khỏi `secrets.env`; slot không tồn tại coi như xong.
    ///
    /// # Errors
    /// [`RouterError::SecretUnavailable`] khi thiếu khoá; lỗi đĩa bọc cùng kiểu.
    pub(crate) fn remove_profile_secret(&self, env_name: &str) -> Result<(), RouterError> {
        let Some(store) = self.secret_store() else {
            return Err(RouterError::SecretUnavailable);
        };
        let registry =
            bean_settings::SecretSlotRegistry::from_names(std::slice::from_ref(&env_name));
        store
            .remove(&registry, env_name)
            .map_err(|error| RouterError::SecretWrite(error.to_string()))?;
        Ok(())
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

    /// Thêm profile vào `[[llm.providers]]` — **không** switch runtime.
    ///
    /// Thứ tự transactional: normalize → absorb key vào `secrets.env` → validate →
    /// persist `bean.toml` → mutate RAM. Lỗi bất kỳ ⇒ config giữ nguyên.
    ///
    /// # Errors
    /// * [`RouterError::ProfileInvalid`] — tên/trường không đạt ràng buộc.
    /// * [`RouterError::ProfileExists`] — đã có profile cùng tên.
    /// * [`RouterError::SecretUnavailable`] / [`RouterError::SecretWrite`] — không ghi được key.
    /// * [`RouterError::ConfigPersist`] — không ghi được `bean.toml`.
    /// * [`RouterError::StatePoisoned`] — lock bị poison.
    pub fn add_llm_provider(
        &self,
        mut profile: bean_types::config::LlmProfile,
    ) -> Result<LlmRuntimeConfig, RouterError> {
        Self::normalize_allowed_models(&mut profile);
        self.absorb_profile_api_key(&mut profile)?;
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
    /// * [`RouterError::SecretUnavailable`] / [`RouterError::SecretWrite`] — thao tác key.
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
                // M29: key hiện hành (env/secrets.env) giữ nguyên. Legacy: profile cũ
                // còn key trong toml thì di chuyển sang secrets.env **nếu slot còn
                // trống** (env/file đang thắng thì không đè), để persist — vốn không
                // ghi `api_key` nữa — không làm mất key.
                if let Some(legacy) =
                    Self::legacy_stored_api_key(&config.llm.providers[existing_index])
                {
                    if self.secret_store().is_some() {
                        if profile.api_key_env.trim().is_empty() {
                            profile.api_key_env = Self::derive_env_name(name);
                        }
                        let snapshot = self.secret_snapshot();
                        let lookup = bean_settings::env_first_lookup(snapshot.as_ref());
                        if lookup(&profile.api_key_env).is_none() {
                            self.write_profile_secret(&profile.api_key_env, &legacy)?;
                        }
                    } else {
                        // Không có khoá (test/`chat`): giữ hành vi cũ trong RAM.
                        profile.api_key = Some(legacy);
                    }
                }
            }
            ApiKeyUpdate::Clear => {
                if !profile.api_key_env.trim().is_empty() {
                    self.remove_profile_secret(&profile.api_key_env)?;
                }
            }
            ApiKeyUpdate::Set(key) => {
                let trimmed = key.trim();
                if trimmed.is_empty() {
                    // Nhất quán semantics cũ (normalize_api_key): dán chỉ khoảng trắng
                    // là ý định xoá — không bao giờ lưu slot giá trị trắng.
                    if !profile.api_key_env.trim().is_empty() {
                        self.remove_profile_secret(&profile.api_key_env)?;
                    }
                } else {
                    if profile.api_key_env.trim().is_empty() {
                        profile.api_key_env = Self::derive_env_name(name);
                    }
                    self.write_profile_secret(&profile.api_key_env, trimmed)?;
                }
            }
        }
        profile.api_key = None;
        profile
            .validate_for_storage()
            .map_err(|error| RouterError::ProfileInvalid(error.to_string()))?;
        let active = config.llm.active_profile_name() == name;
        let provider = if active {
            let api_key = self.resolve_profile_api_key(&profile);
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
