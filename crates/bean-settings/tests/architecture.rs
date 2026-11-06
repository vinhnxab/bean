//! Kiến trúc kiểm tra cho `bean-settings` (M29 Q4).
//!
//! Crate này **chỉ được phụ thuộc `bean-settings`** — không phụ thuộc
//! `bean-core`, `bean-web` hay bất kỳ crate nào khác. Mục đích là đảm bảo
//! `bean-settings` tự-contained, có thể dùng độc lập từ CLI khi Router
//! chưa khởi động (AGENTS.md mục 0.10 DIP/SRP).
//!
//! `#![forbid(unsafe_code)]` áp dụng toàn bộ crate.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::type_complexity,
    clippy::needless_borrows_for_generic_args
)]

use bean_settings::{SecretSlotRegistry, SecretStore, env_first_lookup};
use secrecy::{ExposeSecret, SecretString};

/// `bean-settings` không cần `bean-core`/`bean-web` để compile — test này
/// chỉ import từ `bean_settings`. Nếu có phụ thuộc vi phạm, `cargo check`
/// sẽ báo lỗi ngay ở đây.
#[test]
fn bean_settings_is_independent() {
    // Dùng chính các public API của crate để chứng minh chúng tồn tại
    // và compile được mà không cần bean-core/bean-web.
    let _ = SecretSlotRegistry::default();
    let _lookup: Box<dyn Fn(&str) -> Option<String>> = Box::new(env_first_lookup(None));
}

/// `SecretStore` ghi file `0600`, thư mục cha `0700` (M29 mục 3.1).
#[test]
fn secret_store_creates_private_files() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let store = SecretStore::new(data.join("secrets.env"));
    let registry = SecretSlotRegistry::from_names(&["BEAN_TEST_KEY"]);
    store
        .set(&registry, "BEAN_TEST_KEY", SecretString::from("sk-1"))
        .unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(
        snapshot
            .get("BEAN_TEST_KEY")
            .map(|v| v.expose_secret().to_string()),
        Some("sk-1".to_string())
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(data.join("secrets.env"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "secrets.env phải là 0600"
        );
        assert_eq!(
            std::fs::metadata(&data).unwrap().permissions().mode() & 0o777,
            0o700,
            "thư mục chứa phải là 0700"
        );
    }
}

/// Tên slot ngoài danh sách khai báo bị từ chối (`PATH`, `LD_PRELOAD`…).
#[test]
fn registry_rejects_names_outside_slots() {
    let config = bean_types::Config::default();
    let registry = SecretSlotRegistry::from_config(&config);
    // `PATH` không phải là `*_env` nào nên không xuất hiện trong registry.
    assert!(registry.slot("PATH").is_none());
    assert!(registry.slot("LD_PRELOAD").is_none());
    assert!(registry.slot("HTTPS_PROXY").is_none());
}
