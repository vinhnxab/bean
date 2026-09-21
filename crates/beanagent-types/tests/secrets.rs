//! Test cho phần secret: `resolve_secrets_with` (agents.md mục 20: "thiếu env";
//! mục 15.6: secret không được lộ qua Debug/log).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use beanagent_types::config::{Config, ConfigError, WebSearchProvider};
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
///
/// Cần thiết vì cấu hình mặc định bật `web` + `tavily`, mà nhánh đó lại đòi `TAVILY_API_KEY`
/// trước cả key của LLM — sẽ làm test về secret trở nên khó đọc.
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
fn searches_provider_rules_for_api_key() {
    // Tavily: cần key khi nhóm `web` được bật (mặc định bật).
    let err = Config::default()
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "k")]))
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::MissingEnv { ref env, .. } if env == "TAVILY_API_KEY"),
        "nhận {err:?}"
    );

    // SearXNG tự host: không cần key.
    let mut config = Config::default();
    config.tools.web_search.provider = WebSearchProvider::Searxng;
    let secrets = config
        .resolve_secrets_with(env_from(&[("ANTHROPIC_API_KEY", "k")]))
        .unwrap();
    assert!(secrets.web_search_api_key.is_none());
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
