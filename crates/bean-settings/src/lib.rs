//! # bean-settings — kho secret và dịch vụ thiết lập thống nhất (M29)
//!
//! Trước M29 một khoá bí mật có thể nằm ở bốn nơi (biến môi trường,
//! `~/.config/bean/env`, `bean.toml`, `voice-state.json`) và nơi nào "thắng" phụ thuộc
//! loại khoá (Q6/K28). M29 quy về **một nơi lưu**: `data.dir/secrets.env` (quyền `0600`,
//! thư mục `0700`), nhập được từ cả UI lẫn terminal, luôn đi qua [`SecretStore`]
//! (D19.1 trong `docs/decisions.md`).
//!
//! * [`slot`] — registry "secret slot": danh sách tên secret **hợp lệ**, dẫn xuất từ
//!   cấu hình (`*_env` của llm/profile/voice/telegram/search/billing/marketing). Ghi
//!   giá trị cho tên ngoài slot bị từ chối — `secrets.env` không thể trở thành đường
//!   chèn biến môi trường tuỳ ý (`PATH`, `LD_PRELOAD`, `HTTPS_PROXY`…).
//! * [`store`] — đọc/ghi `secrets.env`: parse `NAME=value`, giá trị ≤ 4096 byte và
//!   không chứa ký tự điều khiển, ghi atomic (tmp tạo ngay với quyền `0600` → `fsync`
//!   → `rename`) dưới khoá file (`fd-lock`) để CLI và server không ghi chồng nhau.
//! * [`env_first_lookup`] — cầu nối sang `Config::resolve_*_with` của `bean-types`
//!   với thứ tự ưu tiên một chiều D19.3: **biến môi trường tiến trình > `secrets.env`
//!   > chưa đặt**.
//!
//! Secret sống trong bộ nhớ dưới dạng `secrecy::SecretString`; Bean **không bao giờ**
//! đẩy giá trị vào môi trường tiến trình, nên tiến trình con (MCP stdio, sandbox,
//! scanner, Chrome) không thừa hưởng.
//!
//! * [`probe`] — probe "kiểm tra kết nối" dùng chung cho `bean doctor --online` và
//!   `POST /api/settings/test/{target}` (M29 mục 8.1): một nguồn sự thật, mọi thông
//!   điệp đã redact secret.

pub mod error;
pub mod probe;
pub mod settings;
pub mod slot;
pub mod store;

pub use error::SettingsError;
pub use probe::{
    ProbeLevel, ProbeOutcome, online_client, probe_llm, probe_search, probe_telegram,
    probe_voice_api, searxng_search_url,
};
pub use settings::{
    SETTINGS, SetOutcome, SettingApply, SettingEntry, SettingKind, SettingRead, SettingSource,
    SettingStore, SettingTier, SettingValue, SettingsService, lookup, parse_value,
    value_from_config,
};
pub use slot::{SecretSlot, SecretSlotRegistry, SlotGroup, is_env_name};
pub use store::{MAX_VALUE_BYTES, SecretSnapshot, SecretStore};

/// Tra cứu secret theo thứ tự ưu tiên D19.3 cho các `Config::resolve_*_with`.
///
/// Kết quả trả về là closure `Fn(&str) -> Option<String>` — đúng hình dạng `get_env`
/// mà `resolve_secrets_with` chấp nhận; giá trị được bọc lại thành `SecretString`
/// ngay bên trong resolver nên `String` trung gian không thoát ra ngoài.
///
/// Biến môi trường **đặt nhưng rỗng** được coi như chưa đặt (rơi xuống `secrets.env`):
/// `export FOO=` của shell không nên che mất key đã lưu trong file — hành vi ngược
/// với "file thắng env" cũ chỉ vì một biến rỗng là bẫy âm thầm (K28).
pub fn env_first_lookup<'a>(
    snapshot: Option<&'a SecretSnapshot>,
) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => snapshot.and_then(|store| store.lookup_str(name)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use secrecy::{ExposeSecret, SecretString};

    #[test]
    fn lookup_prefers_snapshot_when_env_absent() {
        let mut config = bean_types::config::Config::default();
        config.llm.api_key_env = "BEAN_TEST_LOOKUP_KEY".to_string();
        let registry = SecretSlotRegistry::from_config(&config);
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.env"));
        store
            .set(
                &registry,
                "BEAN_TEST_LOOKUP_KEY",
                SecretString::from("from-file"),
            )
            .unwrap();
        let snapshot = store.snapshot().unwrap();

        let lookup = env_first_lookup(Some(&snapshot));
        // Biến chưa đặt ⇒ lấy từ file. (`set_var` là unsafe trong Rust 2024 nên trường
        // hợp env thắng không test được trong tiến trình này — ghi rõ ở docs.)
        assert_eq!(lookup("BEAN_TEST_LOOKUP_KEY").as_deref(), Some("from-file"));
        assert_eq!(lookup("BEAN_TEST_LOOKUP_MISSING"), None);
    }

    #[test]
    fn lookup_without_snapshot_is_env_only() {
        let lookup = env_first_lookup(None);
        assert_eq!(lookup("BEAN_TEST_LOOKUP_MISSING"), None);
    }

    #[test]
    fn snapshot_values_are_secretstring_and_readable_once() {
        let mut config = bean_types::config::Config::default();
        config.voice.api_key_env = "BEAN_TEST_VOICE_KEY".to_string();
        let registry = SecretSlotRegistry::from_config(&config);
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("nested").join("secrets.env"));
        store
            .set(
                &registry,
                "BEAN_TEST_VOICE_KEY",
                SecretString::from("sk-123"),
            )
            .unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(
            snapshot
                .get("BEAN_TEST_VOICE_KEY")
                .map(|value| value.expose_secret().to_string()),
            Some("sk-123".to_string())
        );
        assert!(snapshot.contains("BEAN_TEST_VOICE_KEY"));
        assert!(!snapshot.contains("BEAN_TEST_VOICE_MISSING"));
    }
}
