//! Test CRUD provider profile (Giai đoạn 3): thêm/sửa/xoá qua Router với persist
//! `bean.toml` bằng `toml_edit`, chặn profile active/default, và các đường lỗi
//! đĩa — không mạng, không key thật.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use async_trait::async_trait;
use bean_core::RouterError;
use bean_core::router::{ApiKeyUpdate, Router, RouterDeps, RouterOptions};
use bean_core::store::MemoryStore;
use bean_llm::{ChatRequest, LlmError, LlmProvider};
use bean_security::CapWorkspace;
use bean_tools::ToolRegistry;
use bean_types::Config;
use bean_types::LlmResponse;
use bean_types::config::{LlmProfile, LlmProviderKind};
use secrecy::{ExposeSecret, SecretString};

/// Provider test phân biệt được qua `name()`.
#[derive(Debug)]
struct TagProvider {
    tag: &'static str,
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for TagProvider {
    async fn chat(&self, _request: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(LlmResponse::text_only(self.tag))
    }

    fn name(&self) -> &'static str {
        self.tag
    }
}

/// Factory test: trả provider theo tag; `fail = true` mô phỏng thiếu key; ghi lại
/// model của candidate để chứng minh apply_profile chạy với trường mới.
struct RecordingFactory {
    tag: &'static str,
    fail: bool,
    seen_models: Mutex<Vec<String>>,
}

impl RecordingFactory {
    fn build(
        &self,
        config: &bean_types::config::LlmConfig,
        _key: Option<SecretString>,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if self.fail {
            return Err(LlmError::Config(format!(
                "provider cần API key — đặt nó trong biến môi trường `{}`",
                config.api_key_env
            )));
        }
        self.seen_models
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(config.model.clone());
        Ok(Arc::new(TagProvider {
            tag: self.tag,
            calls: AtomicUsize::new(0),
        }))
    }
}

/// File cấu hình ban đầu — nguồn sự thật cho cả `Config::load` lẫn persist.
const INITIAL_TOML: &str = r#"[llm]
provider = "anthropic"
model = "claude-sonnet-5"
allowed_models = ["claude-sonnet-5"]
api_key_env = "BEAN_TEST_DEFAULT_KEY"

[[llm.providers]]
name = "ollama"
provider = "openai_compat"
model = "qwen3:14b"
allowed_models = ["qwen3:14b", "qwen3:32b"]
base_url = "http://127.0.0.1:9/v1"
"#;

struct Setup {
    _temp: tempfile::TempDir,
    router: Router,
    config_path: std::path::PathBuf,
}

/// Profile ollama chuẩn của setup — dùng cho cả TOML và nhánh config-from-code.
fn ollama_profile() -> LlmProfile {
    LlmProfile {
        base_url: Some("http://127.0.0.1:9/v1".to_string()),
        allowed_models: vec!["qwen3:14b".to_string(), "qwen3:32b".to_string()],
        ..new_profile("ollama", "qwen3:14b")
    }
}

/// Dựng Router biết đường `bean.toml` (CRUD bật). Khi `config_is_directory = true`,
/// `config_path` trỏ vào THƯ MỤC để mô phỏng lỗi đĩa khi persist.
fn setup(config_is_directory: bool) -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("bean.toml");
    let mut config = if config_is_directory {
        // Đọc file sẽ fail (là thư mục) — dựng config trong code, tương đương TOML.
        std::fs::create_dir_all(&config_path).unwrap();
        let mut config = Config::default();
        config.llm.provider = LlmProviderKind::Anthropic;
        config.llm.model = "claude-sonnet-5".to_string();
        config.llm.allowed_models = vec!["claude-sonnet-5".to_string()];
        config.llm.providers = vec![ollama_profile()];
        config
    } else {
        std::fs::write(&config_path, INITIAL_TOML).unwrap();
        Config::load(&config_path).unwrap()
    };
    config.agent.allowed_users = vec!["telegram:42".to_string()];
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_root).unwrap());
    let router = Router::with_options(
        RouterDeps {
            config_path: Some(config_path.clone()),
            default_profile: config.llm.as_profile(),
            config,
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(ToolRegistry::with_workspace(workspace)),
            llm: Arc::new(TagProvider {
                tag: "start",
                calls: AtomicUsize::new(0),
            }),
            audit: None,
            skills_index: String::new(),
            skills: None,
            llm_state_path: None,
        },
        RouterOptions::default(),
    );
    Setup {
        _temp: temp,
        router,
        config_path,
    }
}

/// Router không biết đường config — CRUD vẫn chạy trên RAM (`config_path: None`).
fn setup_ram_only() -> (tempfile::TempDir, Router) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::load(Path::new("Cargo.toml")).unwrap_or_default();
    config.llm.providers = vec![LlmProfile {
        name: "ollama".to_string(),
        provider: LlmProviderKind::OpenAiCompat,
        model: "qwen3:14b".to_string(),
        allowed_models: vec!["qwen3:14b".to_string()],
        api_key: None,
        api_key_env: String::new(),
        base_url: None,
        max_tokens: 4096,
    }];
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_root).unwrap());
    let router = Router::with_options(
        RouterDeps {
            config_path: None,
            default_profile: config.llm.as_profile(),
            config,
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(ToolRegistry::with_workspace(workspace)),
            llm: Arc::new(TagProvider {
                tag: "start",
                calls: AtomicUsize::new(0),
            }),
            audit: None,
            skills_index: String::new(),
            skills: None,
            llm_state_path: None,
        },
        RouterOptions::default(),
    );
    (temp, router)
}

fn new_profile(name: &str, model: &str) -> LlmProfile {
    LlmProfile {
        name: name.to_string(),
        provider: LlmProviderKind::OpenAiCompat,
        model: model.to_string(),
        allowed_models: vec![model.to_string()],
        api_key: None,
        api_key_env: String::new(),
        base_url: None,
        max_tokens: 4096,
    }
}

fn count_provider_entries(raw: &str) -> usize {
    raw.matches("[[llm.providers]]").count()
}

#[test]
fn add_persists_toml_and_registers() {
    let setup = setup(false);
    let profiles = setup.router.llm_profiles().unwrap();
    assert_eq!(profiles.len(), 2); // default + ollama

    setup
        .router
        .add_llm_provider(new_profile("openrouter", "anthropic/claude-sonnet-5"))
        .unwrap();

    // RAM có profile mới, chưa active.
    let profiles = setup.router.llm_profiles().unwrap();
    assert_eq!(profiles.len(), 3);
    let added = profiles.iter().find(|p| p.name == "openrouter").unwrap();
    assert_eq!(added.model, "anthropic/claude-sonnet-5");
    assert!(!added.active);

    // Đĩa: parse lại bằng Config::load — 2 entry trong mảng + giữ phần [llm].
    let config = Config::load(&setup.config_path).unwrap();
    assert_eq!(config.llm.providers.len(), 2);
    assert_eq!(config.llm.providers[1].name, "openrouter");
    assert_eq!(config.llm.model, "claude-sonnet-5");
    let raw = std::fs::read_to_string(&setup.config_path).unwrap();
    assert_eq!(count_provider_entries(&raw), 2);
}

#[test]
fn add_duplicate_fails_and_keeps_disk_unchanged() {
    let setup = setup(false);
    let error = setup
        .router
        .add_llm_provider(new_profile("ollama", "other"))
        .unwrap_err();
    assert!(matches!(error, RouterError::ProfileExists(_)));
    assert_eq!(error.code(), "profile_exists");
    // RAM + đĩa giữ nguyên.
    assert_eq!(setup.router.llm_profiles().unwrap().len(), 2);
    let config = Config::load(&setup.config_path).unwrap();
    assert_eq!(config.llm.providers.len(), 1);
}

#[test]
fn add_reserved_default_name_fails() {
    let setup = setup(false);
    let error = setup
        .router
        .add_llm_provider(new_profile("default", "m"))
        .unwrap_err();
    assert!(matches!(error, RouterError::ProfileInvalid(_)));
    assert_eq!(error.code(), "profile_invalid");
}

#[test]
fn add_invalid_profile_fails() {
    let setup = setup(false);
    let mut invalid = new_profile("bad", "m");
    invalid.max_tokens = 0;
    let error = setup.router.add_llm_provider(invalid).unwrap_err();
    assert!(matches!(error, RouterError::ProfileInvalid(_)));
    // Không ghi đĩa khi validate fail.
    assert_eq!(
        count_provider_entries(&std::fs::read_to_string(&setup.config_path).unwrap()),
        1
    );
}

#[test]
fn update_persists_and_reflects() {
    let setup = setup(false);
    let mut updated = new_profile("ollama", "qwen3:32b");
    updated.allowed_models = vec!["qwen3:32b".to_string()];
    updated.base_url = Some("http://127.0.0.1:9/v2".to_string());

    setup
        .router
        .update_llm_provider("ollama", updated, ApiKeyUpdate::Keep)
        .unwrap();

    let profiles = setup.router.llm_profiles().unwrap();
    let ollama = profiles.iter().find(|p| p.name == "ollama").unwrap();
    assert_eq!(ollama.model, "qwen3:32b");
    assert_eq!(ollama.base_url.as_deref(), Some("http://127.0.0.1:9/v2"));

    let config = Config::load(&setup.config_path).unwrap();
    assert_eq!(config.llm.providers[0].model, "qwen3:32b");
    // Runtime không đổi vì profile chưa active.
    assert_eq!(setup.router.llm_config().unwrap().model, "claude-sonnet-5");
}

#[test]
fn update_active_profile_rebuilds_provider() {
    let setup = setup(false);
    let switch_factory = RecordingFactory {
        tag: "switch",
        fail: false,
        seen_models: Mutex::new(Vec::new()),
    };
    setup
        .router
        .switch_llm_provider_with("ollama", None, &|config, key| {
            switch_factory.build(config, key)
        })
        .unwrap();
    assert_eq!(setup.router.llm_config().unwrap().model, "qwen3:14b");

    let rebuild_factory = RecordingFactory {
        tag: "rebuilt",
        fail: false,
        seen_models: Mutex::new(Vec::new()),
    };
    setup
        .router
        .update_llm_provider_with(
            "ollama",
            new_profile("ollama", "qwen3:32b"),
            ApiKeyUpdate::Keep,
            &|config, key| rebuild_factory.build(config, key),
        )
        .unwrap();

    // Runtime chuyển sang hình thái mới ngay: model runtime mới + factory được
    // dựng với candidate chứa trường mới.
    assert_eq!(setup.router.llm_config().unwrap().model, "qwen3:32b");
    assert_eq!(
        rebuild_factory.seen_models.lock().unwrap().as_slice(),
        ["qwen3:32b"]
    );
    // Đĩa cũng cập nhật.
    let config = Config::load(&setup.config_path).unwrap();
    assert_eq!(config.llm.providers[0].model, "qwen3:32b");
}

#[test]
fn update_active_with_failing_factory_keeps_old_state() {
    let setup = setup(false);
    let ok_factory = RecordingFactory {
        tag: "ok",
        fail: false,
        seen_models: Mutex::new(Vec::new()),
    };
    setup
        .router
        .switch_llm_provider_with("ollama", None, &|config, key| ok_factory.build(config, key))
        .unwrap();

    let fail_factory = RecordingFactory {
        tag: "fail",
        fail: true,
        seen_models: Mutex::new(Vec::new()),
    };
    let error = setup
        .router
        .update_llm_provider_with(
            "ollama",
            new_profile("ollama", "qwen3:32b"),
            ApiKeyUpdate::Keep,
            &|config, key| fail_factory.build(config, key),
        )
        .unwrap_err();
    assert!(matches!(error, RouterError::LlmProvider(_)));
    assert_eq!(error.code(), "provider_build_failed");

    // Transactional: RAM + đĩa đều nguyên trạng cũ.
    assert_eq!(setup.router.llm_config().unwrap().model, "qwen3:14b");
    let config = Config::load(&setup.config_path).unwrap();
    assert_eq!(config.llm.providers[0].model, "qwen3:14b");
}

#[test]
fn update_default_is_readonly() {
    let setup = setup(false);
    let error = setup
        .router
        .update_llm_provider("default", new_profile("default", "m"), ApiKeyUpdate::Keep)
        .unwrap_err();
    assert!(matches!(error, RouterError::DefaultProfileReadonly));
    assert_eq!(error.code(), "default_readonly");
}

#[test]
fn update_unknown_fails() {
    let setup = setup(false);
    let error = setup
        .router
        .update_llm_provider(
            "khong-ton-tai",
            new_profile("khong-ton-tai", "m"),
            ApiKeyUpdate::Keep,
        )
        .unwrap_err();
    assert!(matches!(error, RouterError::ProfileNotFound(_)));
}

#[test]
fn update_body_name_is_ignored() {
    // Body có tên khác không đổi được tên profile — tên lấy từ tham số.
    let setup = setup(false);
    let mut mismatched = new_profile("tên-khác", "qwen3:32b");
    mismatched.name = "tên-khác".to_string();
    setup
        .router
        .update_llm_provider("ollama", mismatched, ApiKeyUpdate::Keep)
        .unwrap();
    let profiles = setup.router.llm_profiles().unwrap();
    assert!(profiles.iter().any(|p| p.name == "ollama"));
    assert!(!profiles.iter().any(|p| p.name == "tên-khác"));
}

#[test]
fn delete_persists_and_removes() {
    let setup = setup(false);
    setup.router.delete_llm_provider("ollama").unwrap();

    let profiles = setup.router.llm_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "default");

    let config = Config::load(&setup.config_path).unwrap();
    assert!(config.llm.providers.is_empty());
    let raw = std::fs::read_to_string(&setup.config_path).unwrap();
    assert_eq!(count_provider_entries(&raw), 0);
    // Phần [llm] gốc vẫn còn.
    assert!(raw.contains("[llm]"));
}

#[test]
fn delete_active_profile_is_blocked() {
    let setup = setup(false);
    let factory = RecordingFactory {
        tag: "x",
        fail: false,
        seen_models: Mutex::new(Vec::new()),
    };
    setup
        .router
        .switch_llm_provider_with("ollama", None, &|config, key| factory.build(config, key))
        .unwrap();

    let error = setup.router.delete_llm_provider("ollama").unwrap_err();
    assert!(matches!(error, RouterError::ProfileActive(_)));
    assert_eq!(error.code(), "profile_active");
    // Không xoá trên RAM lẫn đĩa.
    assert!(
        setup
            .router
            .llm_profiles()
            .unwrap()
            .iter()
            .any(|p| p.name == "ollama")
    );
    let config = Config::load(&setup.config_path).unwrap();
    assert_eq!(config.llm.providers.len(), 1);
}

#[test]
fn delete_default_is_readonly() {
    let setup = setup(false);
    let error = setup.router.delete_llm_provider("default").unwrap_err();
    assert!(matches!(error, RouterError::DefaultProfileReadonly));
}

#[test]
fn delete_unknown_fails() {
    let setup = setup(false);
    let error = setup
        .router
        .delete_llm_provider("khong-ton-tai")
        .unwrap_err();
    assert!(matches!(error, RouterError::ProfileNotFound(_)));
}

#[test]
fn persist_failure_is_config_unwritable_and_ram_unchanged() {
    let setup = setup(true); // config_path là THƯ MỤC — persist fail lúc đọc file
    let error = setup
        .router
        .add_llm_provider(new_profile("openrouter", "m"))
        .unwrap_err();
    assert!(matches!(error, RouterError::ConfigPersist(_)));
    assert_eq!(error.code(), "config_unwritable");
    // Transactional: RAM giữ nguyên.
    assert_eq!(setup.router.llm_profiles().unwrap().len(), 2);
}

#[test]
fn config_path_none_is_ram_only() {
    let (_temp, router) = setup_ram_only();
    // CLI/test không biết đường config: CRUD không ghi đĩa nhưng vẫn chạy trên RAM.
    assert!(!router.llm_profiles_writable());
    router.add_llm_provider(new_profile("local", "m")).unwrap();
    let profiles = router.llm_profiles().unwrap();
    assert!(profiles.iter().any(|p| p.name == "local"));
}

/// Chuẩn hoá `allowed_models` lúc lưu: lọc phần tử rỗng và tự chèn model mặc định
/// vào đầu khi thiếu — bảo đảm `serve` nạp config nổi (validate.rs từ chối khởi
/// động khi model mặc định vắng mặt trong danh sách).
#[test]
fn add_profile_normalizes_allowed_models() {
    let setup = setup(false);
    let mut profile = new_profile("groq", "llama-4-8b");
    // Người dùng dán thiếu dấu `,` (tạo phần tử "llama-4-8b;") + phần tử rỗng.
    profile.allowed_models = vec!["llama-4-8b;".to_string(), " ".to_string()];
    setup.router.add_llm_provider(profile).unwrap();

    let info = setup
        .router
        .llm_profiles()
        .unwrap()
        .into_iter()
        .find(|p| p.name == "groq")
        .unwrap();
    assert_eq!(info.allowed_models, vec!["llama-4-8b", "llama-4-8b;"]);

    // File persist chứa danh sách đã chuẩn hoá (model mặc định đứng đầu).
    let stored = std::fs::read_to_string(&setup.config_path).unwrap();
    assert!(stored.contains("allowed_models = [\"llama-4-8b\", \"llama-4-8b;\"]"));
}

/// Factory ghi lại API key nhận được để chứng minh thứ tự ưu tiên resolve.
struct KeyCaptureFactory {
    seen_keys: Mutex<Vec<Option<String>>>,
}

impl KeyCaptureFactory {
    fn build(
        &self,
        _config: &bean_types::config::LlmConfig,
        key: Option<SecretString>,
    ) -> Result<Arc<dyn LlmProvider>, LlmError> {
        self.seen_keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(key.as_ref().map(|k| k.expose_secret().to_string()));
        Ok(Arc::new(TagProvider {
            tag: "key-capture",
            calls: AtomicUsize::new(0),
        }))
    }
}

#[test]
fn stored_key_thang_env_va_hint_4_ky_tu_cuoi() {
    let setup = setup(false);
    let mut with_key = new_profile("paid", "anthropic/claude-opus-5");
    with_key.provider = LlmProviderKind::Anthropic;
    with_key.api_key_env = "BEAN_TEST_PAID_KEY_UNSET".to_string();
    with_key.api_key = Some("sk-file-key-abcd".to_string());
    setup.router.add_llm_provider(with_key).unwrap();

    // Danh sách profile: key_configured = true vì key nằm trong file, hint = 4
    // ký tự cuối, và không bao giờ lộ key nguyên vẹn.
    let profiles = setup.router.llm_profiles().unwrap();
    let paid = profiles.iter().find(|p| p.name == "paid").unwrap();
    assert!(paid.key_configured);
    assert_eq!(paid.api_key_hint.as_deref(), Some("••••abcd"));
    let raw = std::fs::read_to_string(&setup.config_path).unwrap();
    assert!(raw.contains("api_key = \"sk-file-key-abcd\""));
    assert!(!raw.contains("api_key_hint"));

    // Env biến KHÔNG tồn tại mà resolve vẫn ra key file ⇒ đường file được ưu tiên
    // trước đường env (file-key trả về dù env vắng).
    let capture = KeyCaptureFactory {
        seen_keys: Mutex::new(Vec::new()),
    };
    setup
        .router
        .switch_llm_provider_with("paid", None, &|config, key| capture.build(config, key))
        .unwrap();
    assert_eq!(
        capture.seen_keys.lock().unwrap().as_slice(),
        [Some("sk-file-key-abcd".to_string())]
    );
}

#[test]
fn env_thieu_tra_none_khi_khong_co_key_file() {
    let setup = setup(false);
    // Profile không key file + biến env chưa từng tồn tại ⇒ resolve None. Đường
    // env được consult sau đường file; giá trị env thật không test được vì
    // workspace `forbid(unsafe_code)` chặn set_var trong edition 2024.
    let mut profile_env = new_profile("paid-env", "claude-opus-5");
    profile_env.provider = LlmProviderKind::Anthropic;
    profile_env.api_key_env = "BEAN_TEST_MISSING_XYZ_9ZQ".to_string();
    setup.router.add_llm_provider(profile_env).unwrap();
    let capture = KeyCaptureFactory {
        seen_keys: Mutex::new(Vec::new()),
    };
    setup
        .router
        .switch_llm_provider_with("paid-env", None, &|config, key| capture.build(config, key))
        .unwrap();
    assert_eq!(capture.seen_keys.lock().unwrap().as_slice(), [None]);
}

#[test]
fn update_key_set_keep_clear() {
    let setup = setup(false);
    let mut paid = new_profile("paid", "claude-opus-5");
    paid.provider = LlmProviderKind::Anthropic;
    paid.api_key_env = "BEAN_TEST_PAID_KEY_UNSET".to_string();
    setup.router.add_llm_provider(paid).unwrap();

    // SET: dán key mới qua update.
    let mut updated = new_profile("paid", "claude-opus-5");
    updated.provider = LlmProviderKind::Anthropic;
    updated.api_key_env = "BEAN_TEST_PAID_KEY_UNSET".to_string();
    setup
        .router
        .update_llm_provider("paid", updated, ApiKeyUpdate::Set("sk-new-key-7777".into()))
        .unwrap();
    let raw = std::fs::read_to_string(&setup.config_path).unwrap();
    assert!(raw.contains("api_key = \"sk-new-key-7777\""));
    let profiles = setup.router.llm_profiles().unwrap();
    assert_eq!(
        profiles
            .iter()
            .find(|p| p.name == "paid")
            .unwrap()
            .api_key_hint
            .as_deref(),
        Some("••••7777")
    );

    // KEEP: form để trống ⇒ key cũ phải còn nguyên (không bị xoá).
    let mut edited = new_profile("paid", "claude-opus-5");
    edited.provider = LlmProviderKind::Anthropic;
    edited.api_key_env = "BEAN_TEST_PAID_KEY_UNSET".to_string();
    setup
        .router
        .update_llm_provider("paid", edited, ApiKeyUpdate::Keep)
        .unwrap();
    let raw = std::fs::read_to_string(&setup.config_path).unwrap();
    assert!(
        raw.contains("api_key = \"sk-new-key-7777\""),
        "Keep phải giữ key cũ"
    );

    // CLEAR: nút "Xoá key" ⇒ bỏ trường khỏi TOML, quay lại resolve env.
    let mut cleared = new_profile("paid", "claude-opus-5");
    cleared.provider = LlmProviderKind::Anthropic;
    cleared.api_key_env = "BEAN_TEST_PAID_KEY_UNSET".to_string();
    setup
        .router
        .update_llm_provider("paid", cleared, ApiKeyUpdate::Clear)
        .unwrap();
    let config = Config::load(&setup.config_path).unwrap();
    assert!(config.llm.providers.iter().all(|p| p.api_key.is_none()));
    let profiles = setup.router.llm_profiles().unwrap();
    let paid = profiles.iter().find(|p| p.name == "paid").unwrap();
    assert_eq!(paid.api_key_hint, None);
    // Env không có ⇒ key_configured false (api_key_env không rỗng và key file trống).
    assert!(!paid.key_configured);
}

#[test]
fn update_set_key_rong_bi_chuan_hoa_thanh_clear() {
    // Set chuỗi chỉ khoảng trắng bị router normalize về None — giống Clear.
    let setup = setup(false);
    let mut edited = new_profile("ollama", "qwen3:32b");
    edited.allowed_models = vec!["qwen3:32b".to_string()];
    setup
        .router
        .update_llm_provider("ollama", edited, ApiKeyUpdate::Set("   ".into()))
        .unwrap();
    let config = Config::load(&setup.config_path).unwrap();
    assert!(config.llm.providers.iter().all(|p| p.api_key.is_none()));
}

#[test]
fn key_co_ky_tu_dieu_khien_bi_tu_choi_khi_update() {
    let setup = setup(false);
    let mut bad = new_profile("ollama", "qwen3:32b");
    bad.allowed_models = vec!["qwen3:32b".to_string()];
    let error = setup
        .router
        .update_llm_provider("ollama", bad, ApiKeyUpdate::Set("sk-bad\nkey".into()))
        .unwrap_err();
    assert!(matches!(error, RouterError::ProfileInvalid(_)));
    // Nguyên trạng: TOML không bị ghi.
    assert_eq!(
        count_provider_entries(&std::fs::read_to_string(&setup.config_path).unwrap()),
        1
    );
}
