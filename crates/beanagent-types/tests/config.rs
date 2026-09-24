//! Test cho `config.rs`: parse, kiểm tra ngữ nghĩa, và sự khớp giữa `BeanAgent.example.toml`
//! với các struct (agents.md mục 20: "config (thiếu env, sai kiểu)").
//!
//! File test được phép `unwrap`/`panic` (workspace lint chỉ áp cho code sản phẩm).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use beanagent_types::config::{Config, ConfigError, SandboxMode, WebSearchProvider};

/// Ghi nội dung TOML ra file tạm và trả về (thư mục tạm, đường dẫn file).
fn write_config(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("BeanAgent.toml");
    std::fs::write(&path, body).unwrap();
    (dir, path)
}

/// Đường dẫn tới `BeanAgent.example.toml` ở gốc repo.
fn example_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../BeanAgent.example.toml")
}

#[test]
fn example_file_parses_and_validates() {
    // Test này bắt lỗi "file mẫu lệch với struct" — cực dễ xảy ra khi thêm trường cấu hình.
    let config = Config::load(&example_path()).expect("BeanAgent.example.toml phải nạp được");
    assert_eq!(config.agent.max_steps, 25);
    assert_eq!(
        config.agent.allowed_users,
        vec!["web:admin".to_string(), "cli:local".to_string()]
    );
    assert_eq!(config.llm.provider.as_str(), "anthropic");
    assert_eq!(config.llm.model, "claude-sonnet-5");
    assert_eq!(config.security.sandbox.mode, SandboxMode::Docker);
    assert_eq!(config.security.sandbox.memory, "512m");
    assert!(!config.security.sandbox.network);
    assert_eq!(config.tools.web_search.provider, WebSearchProvider::Tavily);
    assert_eq!(config.mcp_servers.len(), 1);
    assert_eq!(config.mcp_servers[0].name, "example");
    assert!(!config.mcp_servers[0].trust);
}

#[test]
fn missing_file_is_read_error() {
    let err = Config::load(Path::new("/nonexistent/BeanAgent.toml")).unwrap_err();
    assert!(
        matches!(err, ConfigError::Read { .. }),
        "phải là Read, nhận {err:?}"
    );
}

#[test]
fn wrong_type_is_parse_error() {
    let (_dir, path) = write_config("[agent]\nmax_steps = \"hai mươi lăm\"\n");
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(err, ConfigError::Parse { .. }),
        "phải là Parse, nhận {err:?}"
    );
}

#[test]
fn unknown_key_is_rejected() {
    let (_dir, path) = write_config("[agent]\nmax_step = 10\n"); // thiếu `s` — lỗi gõ thường gặp
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(err, ConfigError::Parse { .. }),
        "phải là Parse (deny_unknown_fields), nhận {err:?}"
    );
}

#[test]
fn invalid_semantic_values_are_rejected() {
    let cases = [
        ("[agent]\nmax_steps = 0\n", "max_steps"),
        (
            "[agent]\ncontext_budget_tokens = 10\n",
            "context_budget_tokens",
        ),
        ("[agent]\ntimezone = \"\"\n", "timezone"),
        ("[agent]\nallowed_users = []\n", "allowed_users"),
        (
            "[tools]\nenabled = [\"files\", \"telepathy\"]\n",
            "tools.enabled",
        ),
        (
            "[tools.web_search]\nprovider = \"searxng\"\n",
            "tools.web_search.base_url",
        ),
        (
            "[tools.web_search]\nbase_url = \"file:///tmp/search\"\n",
            "tools.web_search.base_url",
        ),
        ("[llm]\nmodel = \"\"\n", "llm.model"),
        ("[llm]\nmax_tokens = 0\n", "llm.max_tokens"),
        ("[llm]\nallowed_models = [\"khac\"]\n", "allowed_models"),
        ("[security]\ndaily_token_budget = 0\n", "daily_token_budget"),
        (
            "[security.sandbox]\ntimeout_seconds = 0\n",
            "timeout_seconds",
        ),
        ("[security.sandbox]\ncpus = 0.0\n", "cpus"),
        ("[web]\nsession_ttl_hours = 0\n", "session_ttl_hours"),
        ("[web]\npublic_origin = \"ftp://x\"\n", "public_origin"),
        ("[web]\nbind = \"0.0.0.0:7878\"\n", "allow_remote"),
        ("[telegram]\nenabled = true\n", "allowed_user_ids"),
        ("[[mcp_servers]]\nname = \"a b\"\ncommand = \"x\"\n", "name"),
        ("[[mcp_servers]]\nname = \"s\"\ncommand = \"\"\n", "command"),
    ];
    for (body, needle) in cases {
        let (_dir, path) = write_config(body);
        let err = Config::load(&path).unwrap_err();
        match err {
            ConfigError::Invalid(message) => {
                assert!(
                    message.contains(needle),
                    "thông báo `{message}` phải nhắc `{needle}` (body: {body})"
                );
            }
            other => panic!("body {body} phải là Invalid, nhận {other:?}"),
        }
    }
}

#[test]
fn defaults_are_valid_and_public_origin_is_normalized() {
    let mut config = Config::default();
    config.validate().expect("cấu hình mặc định phải hợp lệ");
    assert_eq!(config.web.public_origin, "http://127.0.0.1:7878");

    let (_dir, path) = write_config("[web]\npublic_origin = \"https://agent.example.com/\"\n");
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.web.public_origin, "https://agent.example.com");
    assert_eq!(loaded.web.bind.to_string(), "127.0.0.1:7878");
}

#[test]
fn tilde_is_expanded_in_paths() {
    let home = std::env::var("HOME").unwrap();
    let (_dir, path) =
        write_config("[data]\ndir = \"~/beanagent-test\"\n[agent]\nworkspace = \"/tmp/ws\"\n");
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.data.dir, PathBuf::from(&home).join("beanagent-test"));
    assert_eq!(loaded.agent.workspace, PathBuf::from("/tmp/ws"));
}

#[test]
fn searxng_requires_base_url_only_when_web_tools_are_enabled() {
    let mut disabled = Config::default();
    disabled.tools.enabled.retain(|group| group != "web");
    disabled.tools.web_search.provider = WebSearchProvider::Searxng;
    disabled.tools.web_search.base_url = None;
    disabled
        .validate()
        .expect("SearXNG chưa bật thì không cần endpoint");

    let mut enabled = Config::default();
    enabled.tools.web_search.provider = WebSearchProvider::Searxng;
    enabled.tools.web_search.base_url = None;
    let error = enabled.validate().unwrap_err();
    assert!(matches!(error, ConfigError::Invalid(ref message) if message.contains("base_url")));
}

#[test]
fn duplicate_mcp_names_are_rejected() {
    let body = "[[mcp_servers]]\nname = \"dup\"\ncommand = \"a\"\n[[mcp_servers]]\nname = \"dup\"\ncommand = \"b\"\n";
    let (_dir, path) = write_config(body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("trùng")),
        "nhận {err:?}"
    );
}

#[test]
fn load_or_default_works_without_file() {
    let config = Config::load_or_default(None).unwrap();
    assert_eq!(config.llm.provider.as_str(), "anthropic");
    assert!(config.telegram.token_env.contains("TELEGRAM"));
}
