//! Test cho phần secret: `resolve_secrets_with` (agents.md mục 20: "thiếu env";
//! mục 15.6: secret không được lộ qua Debug/log).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use beanagent_types::config::{Config, ConfigError, LlmProviderKind};
use secrecy::ExposeSecret;

/// Tạo hàm đọc env giả từ danh sách cặp (tên, giá trị).
fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    move |name: &str| map.get(name).cloned()
}

/// Cấu hình cho test, đã tắt nhóm tool `web`.
fn config_without_web_tools() -> Config {
    let mut config = Config::default();
    config.tools.enabled.retain(|group| group != "web");
    config
}

#[test]
fn missing_env_names_the_variable_and_field() {
    let err = Config::default()
        .resolve_secrets_with(|_| None)
        .unwrap_err();
    match err {
        ConfigError::MissingEnv { field, env } => {
            assert_eq!(field, "llm.api_key_env");
            assert_eq!(
                env, "ANTHROPIC_API_KEY",
                "thông báo phải nêu TÊN biến, không nêu giá trị"
            );
        }
        other => panic!("phải là MissingEnv, nhận {other:?}"),
    }
}

#[test]
fn empty_env_value_is_treated_as_missing() {
    let err = Config::default()
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "   ")]))
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::MissingEnv { .. }),
        "nhận {err:?}"
    );
}

#[test]
fn llm_key_resolved_and_never_leaked_by_debug() {
    let secrets = config_without_web_tools()
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "sk-van-de-test")]))
        .unwrap();

    assert!(secrets.has_llm_api_key());
    let key = secrets.llm_api_key.as_ref().expect("phải có key");
    assert_eq!(key.expose_secret(), "sk-van-de-test");

    let dumped = format!("{secrets:?}");
    assert!(
        !dumped.contains("sk-van-de-test"),
        "Debug không được lộ secret: {dumped}"
    );
    assert!(
        dumped.contains("<set>"),
        "Debug nên cho biết biến đã được đặt: {dumped}"
    );
}

#[test]
fn telegram_token_only_required_when_enabled() {
    let mut config = config_without_web_tools();
    let ok = config
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "k")]))
        .unwrap();
    assert!(
        ok.telegram_token.is_none(),
        "khi tắt Telegram thì không đòi token"
    );

    config.telegram.enabled = true;
    config.telegram.allowed_user_ids = vec![42];
    let err = config
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "k")]))
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::MissingEnv { ref env, .. } if env == "TELEGRAM_BOT_TOKEN"),
        "nhận {err:?}"
    );
}

#[test]
fn web_search_key_is_optional_at_startup() {
    // D6.4: thiếu TAVILY_API_KEY **không** chặn khởi động (chat phải chạy được mà không
    // cần Tavily); giá trị được trả về None để M7 báo lỗi tại lúc tool chạy.
    let secrets = Config::default()
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "k")]))
        .unwrap();
    assert!(
        secrets.web_search_api_key.is_none(),
        "thiếu TAVILY_API_KEY phải là None, không phải lỗi"
    );

    // Có key thì được resolve bình thường (kể cả khi nhóm web bật).
    let secrets = Config::default()
        .resolve_secrets_with(env_from(&[
            ("ANTHROPIC_API_KEY", "k"),
            ("TAVILY_API_KEY", "tv-k"),
        ]))
        .unwrap();
    assert_eq!(
        secrets
            .web_search_api_key
            .as_ref()
            .expect("phải có key web_search")
            .expose_secret(),
        "tv-k"
    );
}

#[test]
fn web_search_key_not_required_when_web_group_disabled() {
    let mut config = Config::default();
    config.tools.enabled.retain(|group| group != "web");
    let secrets = config
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "k")]))
        .unwrap();
    assert!(secrets.web_search_api_key.is_none());
}

/// `openai_compat` **có** `base_url` (Ollama/vLLM tự host) không cần API key —
/// `resolve_secrets` phải bỏ qua `llm.api_key_env` (D6.4).
#[test]
fn openai_compat_with_base_url_needs_no_api_key() {
    let mut config = Config::default();
    config.llm.provider = LlmProviderKind::OpenAiCompat;
    config.llm.base_url = Some("http://localhost:11434/v1".to_string());
    let secrets = config.resolve_secrets_with(|_| None).unwrap();
    assert!(
        secrets.llm_api_key.is_none(),
        "Ollama không cần key — phải là None"
    );
}

/// `openai_compat` **không có** `base_url` (mặc định trỏ OpenAI chính thức) thì vẫn
/// fail fast khi thiếu key.
#[test]
fn openai_compat_without_base_url_still_requires_key() {
    let mut config = Config::default();
    config.llm.provider = LlmProviderKind::OpenAiCompat;
    let err = config.resolve_secrets_with(|_| None).unwrap_err();
    assert!(
        matches!(err, ConfigError::MissingEnv { ref env, .. } if env == "ANTHROPIC_API_KEY"),
        "nhận {err:?}"
    );
}
