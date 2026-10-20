//! Test hot-swap provider runtime (Giai đoạn 2): switch profile, persist state,
//! snapshot "default", và các đường lỗi — không mạng, không key thật.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use async_trait::async_trait;
use bean_core::router::{Router, RouterDeps, RouterOptions};
use bean_core::store::MemoryStore;
use bean_core::{Incoming, RouterError};
use bean_llm::{ChatRequest, LlmError, LlmProvider};
use bean_security::CapWorkspace;
use bean_tools::ToolRegistry;
use bean_types::LlmResponse;
use bean_types::config::{DEFAULT_PROFILE_NAME, LlmProfile, LlmProviderKind, LlmState};
use bean_types::{Config, RunEvent};
use secrecy::SecretString;
use tokio::sync::broadcast::Receiver;

/// Provider test phân biệt được qua `name()` và đếm lượt gọi.
#[derive(Debug)]
struct TagProvider {
    tag: &'static str,
    calls: std::sync::atomic::AtomicUsize,
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

/// Factory test: trả provider theo tag; `fail = true` mô phỏng thiếu key.
struct RecordingFactory {
    tag: &'static str,
    fail: bool,
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
        Ok(Arc::new(TagProvider {
            tag: self.tag,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }))
    }
}

/// Cấu hình test: default = anthropic; profile `ollama` = openai_compat không cần key.
fn base_config() -> Config {
    let mut config = Config::default();
    config.agent.allowed_users = vec!["telegram:42".to_string()];
    config.llm.provider = LlmProviderKind::Anthropic;
    config.llm.model = "claude-sonnet-5".to_string();
    config.llm.allowed_models = vec!["claude-sonnet-5".to_string()];
    config.llm.api_key_env = "BEAN_TEST_DEFAULT_KEY".to_string();
    config.llm.base_url = None;
    config.llm.providers = vec![LlmProfile {
        name: "ollama".to_string(),
        provider: LlmProviderKind::OpenAiCompat,
        model: "qwen3:14b".to_string(),
        allowed_models: vec!["qwen3:14b".to_string(), "qwen3:32b".to_string()],
        api_key_env: String::new(),
        base_url: Some("http://127.0.0.1:9/v1".to_string()),
        max_tokens: 4096,
    }];
    // Profile "paid": provider cần key — dùng cho đường lỗi "thiếu key".
    config.llm.providers.push(LlmProfile {
        name: "paid".to_string(),
        provider: LlmProviderKind::Anthropic,
        model: "claude-opus-5".to_string(),
        allowed_models: vec![],
        api_key_env: "BEAN_TEST_PAID_KEY".to_string(),
        base_url: None,
        max_tokens: 4096,
    });
    config
}

/// Dựng Router; state file đặt trong `temp/data/llm-state.json` (nếu `persist = true`).
struct Setup {
    _temp: tempfile::TempDir,
    router: Router,
    state_path: Option<std::path::PathBuf>,
}

fn setup(persist: bool, allowed_models: Option<Vec<String>>) -> Setup {
    let temp = tempfile::tempdir().unwrap();
    let mut config = base_config();
    if let Some(models) = allowed_models {
        config.llm.allowed_models = models;
    }
    let state_path = persist.then(|| bean_types::config::state_path(&temp.path().join("data")));
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
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            audit: None,
            skills_index: String::new(),
            skills: None,
            llm_state_path: state_path.clone(),
        },
        RouterOptions::default(),
    );
    Setup {
        _temp: temp,
        router,
        state_path,
    }
}

async fn wait_final(receiver: &mut Receiver<RunEvent>) -> String {
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(3), receiver.recv())
            .await
            .expect("phải có event trong 3s")
            .expect("broadcast còn mở");
        match event {
            RunEvent::Final { text, .. } => return text,
            RunEvent::Error { code, message, .. } => panic!("run lỗi {code}: {message}"),
            _ => {}
        }
    }
}

async fn run_once(router: &Router, receiver: &mut Receiver<RunEvent>) -> String {
    router
        .submit(Incoming::new("telegram", "42", "telegram:42", "xin chào"))
        .await
        .expect("submit được");
    wait_final(receiver).await
}

#[tokio::test]
async fn switch_provider_swaps_runtime_and_persists_state() {
    let Setup {
        _temp,
        router,
        state_path,
    } = setup(true, None);
    let state_path = state_path.expect("persist = true ⇒ có state path");

    // Run đầu: dùng provider khởi tạo.
    let mut events = router.events();
    assert_eq!(run_once(&router, &mut events).await, "start");

    // Switch sang ollama qua factory test.
    let factory = RecordingFactory {
        tag: "ollama-hot",
        fail: false,
    };
    let after = router
        .switch_llm_provider_with("ollama", None, &|config, key| factory.build(config, key))
        .expect("switch phải thành công");

    assert_eq!(after.provider, LlmProviderKind::OpenAiCompat);
    assert_eq!(after.model, "qwen3:14b");
    assert_eq!(after.base_url.as_deref(), Some("http://127.0.0.1:9/v1"));
    assert!(!after.key_configured, "api_key_env rỗng ⇒ không cần key");

    // State file ghi đúng profile + model.
    let saved = LlmState::load(&state_path).expect("state phải được ghi");
    assert_eq!(saved.profile, "ollama");
    assert_eq!(saved.model, "qwen3:14b");

    // Danh sách profile: ollama active, "default" vẫn là snapshot `[llm]` gốc.
    let profiles = router.llm_profiles().unwrap();
    assert_eq!(profiles.len(), 3);
    assert_eq!(profiles[0].name, DEFAULT_PROFILE_NAME);
    assert!(!profiles[0].active);
    assert_eq!(profiles[0].provider, LlmProviderKind::Anthropic);
    assert_eq!(profiles[1].name, "ollama");
    assert!(profiles[1].active);

    // Run kế tiếp dùng provider MỚI — bằng chứng hot-swap thực sự.
    assert_eq!(run_once(&router, &mut events).await, "ollama-hot");
}

#[tokio::test]
async fn switch_to_default_restores_startup_snapshot() {
    let Setup { router, .. } = setup(false, None);
    let factory = RecordingFactory {
        tag: "ollama-hot",
        fail: false,
    };
    router
        .switch_llm_provider_with("ollama", Some("qwen3:32b"), &|config, key| {
            factory.build(config, key)
        })
        .expect("switch được");
    assert_eq!(router.llm_config().unwrap().model, "qwen3:32b");

    // Switch về "default": quay về snapshot `[llm]` gốc, KHÔNG phải giá trị hiện tại.
    let factory_default = RecordingFactory {
        tag: "anthropic-again",
        fail: false,
    };
    let after = router
        .switch_llm_provider_with(DEFAULT_PROFILE_NAME, None, &|config, key| {
            factory_default.build(config, key)
        })
        .expect("switch về default phải thành công");
    assert_eq!(after.provider, LlmProviderKind::Anthropic);
    assert_eq!(after.model, "claude-sonnet-5");
}

#[tokio::test]
async fn switch_rejects_unknown_profile_and_disallowed_model() {
    let Setup { router, .. } = setup(false, None);
    let factory = RecordingFactory {
        tag: "x",
        fail: false,
    };

    let error = router
        .switch_llm_provider_with("khong-ton-tai", None, &|config, key| {
            factory.build(config, key)
        })
        .unwrap_err();
    assert!(matches!(&error, RouterError::ProfileNotFound(name) if name == "khong-ton-tai"));
    assert_eq!(error.code(), "profile_not_found");

    let error = router
        .switch_llm_provider_with("ollama", Some("model-la"), &|config, key| {
            factory.build(config, key)
        })
        .unwrap_err();
    assert!(matches!(error, RouterError::ModelNotAllowed(_)));
    assert_eq!(error.code(), "model_not_allowed");

    // Trạng thái không đổi sau cả hai lỗi.
    let current = router.llm_config().unwrap();
    assert_eq!(current.provider, LlmProviderKind::Anthropic);
    assert_eq!(current.model, "claude-sonnet-5");
}

#[tokio::test]
async fn switch_failure_keeps_old_provider_and_state() {
    let Setup {
        _temp,
        router,
        state_path,
    } = setup(true, None);
    let state_path = state_path.expect("persist = true ⇒ có state path");

    // Factory lỗi (mô phỏng thiếu key): switch bị từ chối, KHÔNG ghi state.
    let failing = RecordingFactory {
        tag: "x",
        fail: true,
    };
    let error = router
        .switch_llm_provider_with("paid", None, &|config, key| failing.build(config, key))
        .unwrap_err();
    assert!(matches!(error, RouterError::LlmProvider(_)));
    assert_eq!(error.code(), "provider_build_failed");
    // Thông điệp lỗi nêu tên biến env, không chứa giá trị key.
    assert!(error.to_string().contains("BEAN_TEST_PAID_KEY"));

    let current = router.llm_config().unwrap();
    assert_eq!(current.provider, LlmProviderKind::Anthropic);
    assert!(
        LlmState::load(&state_path).is_none(),
        "factory lỗi ⇒ không ghi state"
    );

    // Provider cũ vẫn dùng cho run kế tiếp.
    let mut events = router.events();
    assert_eq!(run_once(&router, &mut events).await, "start");
}

#[tokio::test]
async fn switch_persist_failure_rejects_whole_switch() {
    let temp = tempfile::tempdir().unwrap();
    // State path là một THƯ MỤC: save fail lúc rename ⇒ Persist.
    let state_path = temp.path().join("state.json");
    std::fs::create_dir_all(&state_path).unwrap();
    let config = base_config();
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace = Arc::new(CapWorkspace::open(workspace_root).unwrap());
    let router = Router::with_options(
        RouterDeps {
            config_path: None,
            default_profile: config.llm.as_profile(),
            config: config.clone(),
            store: Arc::new(MemoryStore::new()),
            registry: Arc::new(ToolRegistry::with_workspace(workspace)),
            llm: Arc::new(TagProvider {
                tag: "start",
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            audit: None,
            skills_index: String::new(),
            skills: None,
            llm_state_path: Some(state_path),
        },
        RouterOptions::default(),
    );
    drop(config);

    let factory = RecordingFactory {
        tag: "x",
        fail: false,
    };
    let error = router
        .switch_llm_provider_with("ollama", None, &|config, key| factory.build(config, key))
        .unwrap_err();
    assert!(matches!(error, RouterError::Persist(_)));
    assert_eq!(error.code(), "llm_state_unwritable");
    // Transactional: config giữ nguyên, default vẫn active.
    assert_eq!(
        router.llm_config().unwrap().provider,
        LlmProviderKind::Anthropic
    );
    assert!(router.llm_profiles().unwrap()[0].active);
}

#[tokio::test]
async fn set_llm_model_persists_state_with_active_profile() {
    let Setup {
        _temp,
        router,
        state_path,
    } = setup(
        true,
        Some(vec!["claude-sonnet-5".into(), "claude-opus-5".into()]),
    );
    let state_path = state_path.expect("persist = true ⇒ có state path");

    router
        .set_llm_model("claude-opus-5")
        .expect("model allowed");
    let saved = LlmState::load(&state_path).expect("set_llm_model phải persist");
    assert_eq!(saved.profile, DEFAULT_PROFILE_NAME);
    assert_eq!(saved.model, "claude-opus-5");

    let denied = router.set_llm_model("model-la").unwrap_err();
    assert!(matches!(denied, RouterError::ModelNotAllowed(_)));

    // Sau khi switch profile, set_llm_model persist đúng profile active.
    let factory = RecordingFactory {
        tag: "x",
        fail: false,
    };
    router
        .switch_llm_provider_with("ollama", None, &|config, key| factory.build(config, key))
        .unwrap();
    router
        .set_llm_model("qwen3:32b")
        .expect("model allowed của ollama");
    let saved = LlmState::load(&state_path).unwrap();
    assert_eq!(saved.profile, "ollama");
    assert_eq!(saved.model, "qwen3:32b");
}
