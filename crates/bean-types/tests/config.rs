//! Test cho `config.rs`: parse, kiểm tra ngữ nghĩa, và sự khớp giữa `bean.example.toml`
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

use bean_types::config::{
    Config, ConfigError, QaConfig, QaRunner, QaSandboxConfig, QaSuiteConfig, SandboxMode,
    WebSearchProvider,
};

/// Ghi nội dung TOML ra file tạm và trả về (thư mục tạm, đường dẫn file).
fn write_config(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, body).unwrap();
    (dir, path)
}

/// Đường dẫn tới `bean.example.toml` ở gốc repo.
fn example_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bean.example.toml")
}

#[test]
fn example_file_parses_and_validates() {
    // Test này bắt lỗi "file mẫu lệch với struct" — cực dễ xảy ra khi thêm trường cấu hình.
    let config = Config::load(&example_path()).expect("bean.example.toml phải nạp được");
    assert_eq!(config.agent.max_steps, 25);
    assert_eq!(
        config.agent.allowed_users,
        vec![
            "web:admin".to_string(),
            "cli:local".to_string(),
            "telegram:123456789".to_string(),
        ]
    );
    assert_eq!(config.llm.provider.as_str(), "anthropic");
    assert_eq!(config.llm.model, "claude-sonnet-5");
    assert_eq!(config.security.sandbox.mode, SandboxMode::Docker);
    assert_eq!(config.security.sandbox.memory, "512m");
    assert!(!config.security.sandbox.network);
    assert!(config.learning.enabled);
    assert_eq!(config.learning.min_tool_calls, 5);
    assert_eq!(config.learning.proposal_interval_minutes, 60);
    assert_eq!(config.tools.web_search.provider, WebSearchProvider::Tavily);
    assert_eq!(config.mcp_servers.len(), 1);
    assert_eq!(config.mcp_servers[0].name, "example");
    assert!(!config.mcp_servers[0].trust);
}

#[test]
fn missing_file_is_read_error() {
    let err = Config::load(Path::new("/nonexistent/bean.toml")).unwrap_err();
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
        ("[learning]\nmin_tool_calls = 0\n", "min_tool_calls"),
        (
            "[learning]\nproposal_interval_minutes = 0\n",
            "proposal_interval_minutes",
        ),
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
        (
            "[telegram]\nenabled = true\nallowed_user_ids = [42]\n",
            "agent.allowed_users",
        ),
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
        write_config("[data]\ndir = \"~/bean-test\"\n[agent]\nworkspace = \"/tmp/ws\"\n");
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.data.dir, PathBuf::from(&home).join("bean-test"));
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

// ---------------------------------------------------------------------------
// M23 — `[[infra_scope]]` + `[security_scan]` (kiểm ở tầng load config)
// ---------------------------------------------------------------------------

use bean_types::config::{
    BROWSER_TOOL_GROUP, KNOWN_TOOL_GROUPS, ScanSandboxConfig, ScanScopeEntry, ScanTargetKind,
    SecurityScanConfig,
};

fn scope_entry(kind: ScanTargetKind, value: &str) -> ScanScopeEntry {
    ScanScopeEntry {
        kind,
        value: value.to_string(),
        label: String::new(),
    }
}

/// Mặc định (và cấu hình mẫu) phải là **fail-closed**: scope rỗng, quét tắt.
#[test]
fn infra_scope_is_empty_by_default() {
    let mut config = Config::default();
    assert!(
        config.infra_scope.is_empty(),
        "mặc định KHÔNG được cho phép quét gì cả"
    );
    assert!(!config.security_scan.enabled);
    config
        .validate()
        .expect("scope rỗng là hợp lệ — nghĩa là từ chối mọi thứ");
}

/// Chỉ `ip`/`cidr`; hostname bị từ chối ngay lúc load config.
#[test]
fn hostname_in_infra_scope_is_rejected() {
    let mut config = Config {
        infra_scope: vec![scope_entry(ScanTargetKind::Ip, "mayer.example.com")],
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("hostname")),
        "nhận {err:?}"
    );
}

#[test]
fn cidr_without_prefix_is_rejected() {
    let mut config = Config {
        infra_scope: vec![scope_entry(ScanTargetKind::Cidr, "192.168.10.0")],
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("/len")),
        "{err:?}"
    );
}

#[test]
fn cidr_prefix_beyond_address_size_is_rejected() {
    let mut config = Config {
        infra_scope: vec![scope_entry(ScanTargetKind::Cidr, "192.168.10.0/33")],
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("vượt giới hạn")),
        "{err:?}"
    );
}

#[test]
fn duplicate_infra_scope_entries_are_rejected() {
    let mut config = Config {
        infra_scope: vec![
            scope_entry(ScanTargetKind::Ip, "203.0.113.7"),
            scope_entry(ScanTargetKind::Ip, "203.0.113.7"),
        ],
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("trùng")),
        "{err:?}"
    );
}

#[test]
fn valid_ipv4_and_ipv6_scope_entries_are_accepted() {
    let mut config = Config {
        infra_scope: vec![
            scope_entry(ScanTargetKind::Cidr, "192.168.10.0/24"),
            scope_entry(ScanTargetKind::Ip, "203.0.113.7"),
            scope_entry(ScanTargetKind::Cidr, "fd00::/8"),
        ],
        ..Config::default()
    };
    config.validate().expect("IP/CIDR hợp lệ phải qua kiểm tra");
}

/// D14.3: scanner không có mạng thì vô dụng ⇒ từ chối cấu hình đó.
#[test]
fn scan_sandbox_without_network_is_rejected() {
    let mut config = Config {
        security_scan: SecurityScanConfig {
            enabled: true,
            sandbox: ScanSandboxConfig {
                network: false,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("network")),
        "{err:?}"
    );
}

/// D14.4: chế độ host phải được bật tường minh để không vô tình hạ cấp cách ly.
#[test]
fn host_mode_requires_explicit_allow_host() {
    let mut config = Config {
        security_scan: SecurityScanConfig {
            enabled: true,
            sandbox: ScanSandboxConfig {
                mode: SandboxMode::Host,
                allow_host: false,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("allow_host")),
        "{err:?}"
    );

    // Bật tường minh thì qua.
    config.security_scan.sandbox.allow_host = true;
    config
        .validate()
        .expect("bật allow_host tường minh thì hợp lệ");
}

/// M27: `[[qa.suites]]` rỗng là **hợp lệ** (fail-closed ở tầng tool, y hệt D14.1 của M23).
#[test]
fn empty_qa_suites_is_valid() {
    let mut config = Config {
        qa: QaConfig {
            enabled: true,
            ..Default::default()
        },
        ..Config::default()
    };
    config
        .validate()
        .expect("[[qa.suites]] rỗng phải hợp lệ — tool mới tự từ chối mọi lần gọi");
}

/// M27: suite trùng tên, workdir tuyệt đối/`..`, tên sai ký tự, args rỗng ⇒ đều bị chặn.
#[test]
fn invalid_qa_suite_entries_are_rejected() {
    let cases: Vec<(&str, QaSuiteConfig)> = vec![
        (
            "trùng tên",
            QaSuiteConfig {
                name: "core".to_string(),
                runner: QaRunner::CargoTest,
                workdir: String::new(),
                args: vec![],
            },
        ),
        (
            "workdir tuyệt đối",
            QaSuiteConfig {
                name: "core".to_string(),
                runner: QaRunner::CargoTest,
                workdir: "/etc".to_string(),
                args: vec![],
            },
        ),
        (
            "workdir có ..",
            QaSuiteConfig {
                name: "core".to_string(),
                runner: QaRunner::CargoTest,
                workdir: "../ngoai".to_string(),
                args: vec![],
            },
        ),
        (
            "tên sai ký tự",
            QaSuiteConfig {
                name: "core unit; rm -rf".to_string(),
                runner: QaRunner::CargoTest,
                workdir: String::new(),
                args: vec![],
            },
        ),
        (
            "args rỗng",
            QaSuiteConfig {
                name: "core".to_string(),
                runner: QaRunner::CargoTest,
                workdir: String::new(),
                args: vec!["  ".to_string()],
            },
        ),
    ];
    for (label, suite) in cases {
        let mut config = Config {
            qa: QaConfig {
                enabled: true,
                suites: vec![suite],
                ..Default::default()
            },
            ..Config::default()
        };
        if label == "trùng tên" {
            // Cần hai suite cùng tên để kích hoạt nhánh trùng lặp.
            config.qa.suites.push(config.qa.suites[0].clone());
        }
        let err = config.validate().expect_err(&format!("phải chặn: {label}"));
        assert!(
            matches!(err, ConfigError::Invalid(ref m) if m.contains("qa.suites")),
            "{label}: {err:?}"
        );
    }
}

/// M27 (D17.2): cờ mạng **không phải thứ cấu hình tự do** — `validate()` ghi đè theo runner,
/// nên không cấu hình nào trong `[[qa.suites]]` lật ngược được nó.
#[test]
fn qa_network_flag_is_locked_per_runner() {
    // `cargo_test` cần mạng (CARGO_HOME=/tmp trong container `--rm` ⇒ không có cache).
    let mut cargo = Config {
        qa: QaConfig {
            enabled: true,
            suites: vec![QaSuiteConfig {
                name: "core".to_string(),
                runner: QaRunner::CargoTest,
                workdir: String::new(),
                args: vec![],
            }],
            sandbox: QaSandboxConfig {
                // Cố tình khai `network = false` — phải bị ghi đè thành true.
                network: false,
                ..Default::default()
            },
        },
        ..Config::default()
    };
    cargo.validate().expect("cargo_test phải hợp lệ");
    assert!(
        cargo.qa.sandbox.network,
        "cargo_test bị ép mất mạng ⇒ không tải được crate, D17.2 bị vi phạm"
    );

    // `vitest`/`pytest` chạy offline được ⇒ mặc định KHÔNG mạng.
    let mut offline = Config {
        qa: QaConfig {
            enabled: true,
            suites: vec![QaSuiteConfig {
                name: "web".to_string(),
                runner: QaRunner::Vitest,
                workdir: "web".to_string(),
                args: vec![],
            }],
            sandbox: QaSandboxConfig {
                network: true,
                ..Default::default()
            },
        },
        ..Config::default()
    };
    offline.validate().expect("vitest phải hợp lệ");
    assert!(
        !offline.qa.sandbox.network,
        "vitest phải chạy `--network none` — lỡ tay bật mạng là lỗ hổng"
    );
}

/// M27: timeout 0 hoặc image rỗng là cấu hình vô nghĩa ⇒ chặn lúc nạp.
#[test]
fn qa_sandbox_must_be_usable() {
    let base = QaSuiteConfig {
        name: "core".to_string(),
        runner: QaRunner::Pytest,
        workdir: String::new(),
        args: vec![],
    };
    let mut zero_timeout = Config {
        qa: QaConfig {
            enabled: true,
            suites: vec![base.clone()],
            sandbox: QaSandboxConfig {
                timeout_seconds: 0,
                ..Default::default()
            },
        },
        ..Config::default()
    };
    assert!(matches!(
        zero_timeout.validate().unwrap_err(),
        ConfigError::Invalid(ref m) if m.contains("timeout_seconds")
    ));

    let mut empty_image = Config {
        qa: QaConfig {
            enabled: true,
            suites: vec![base],
            sandbox: QaSandboxConfig {
                image: "  ".to_string(),
                ..Default::default()
            },
        },
        ..Config::default()
    };
    assert!(matches!(
        empty_image.validate().unwrap_err(),
        ConfigError::Invalid(ref m) if m.contains("image")
    ));
}

/// Cảnh báo bật nửa chừng (có channel nhưng thiếu chat_id) là cấu hình im lặng — phải chặn.
#[test]
fn half_configured_alert_is_rejected() {
    let mut config = Config {
        security_scan: SecurityScanConfig {
            enabled: true,
            alert_channel: "telegram".to_string(),
            ..Default::default()
        },
        ..Config::default()
    };
    let err = config.validate().unwrap_err();
    assert!(
        matches!(err, ConfigError::Invalid(ref m) if m.contains("alert_chat_id")),
        "{err:?}"
    );

    config.security_scan.alert_chat_id = "123".to_string();
    config.validate().expect("đủ cặp thì hợp lệ");
}
// ---------------------------------------------------------------------------
// M25 — `[[mcp_clients]]` + `[mcp_server]`
// ---------------------------------------------------------------------------

/// Cấu hình M25 hợp lệ tối thiểu: một client `cline` với role `monitor`.
fn mcp_config(extra: &str) -> String {
    format!(
        r#"
[[roles]]
name = "monitor"
tool_tags = ["infra-read", "memory-read"]

[[roles]]
name = "admin"
tool_tags = ["*"]

[agent]
user_roles = {{ "mcp-client:cline" = "monitor" }}

[[mcp_clients]]
name = "cline"
role = "monitor"

{extra}
"#
    )
}

#[test]
fn valid_mcp_server_config_loads() {
    let (_dir, path) = write_config(&mcp_config("[mcp_server]\nenabled = true\n"));
    let config = Config::load(&path).expect("cấu hình M25 hợp lệ phải nạp được");
    assert!(config.mcp_server.enabled);
    assert!(!config.mcp_server.http_enabled);
    assert_eq!(config.mcp_clients.len(), 1);
    assert_eq!(config.mcp_clients[0].name, "cline");
    assert_eq!(config.mcp_clients[0].role, "monitor");
    assert_eq!(
        Config::mcp_client_identity("cline"),
        "mcp-client:cline".to_string()
    );
    // Tra theo tên dùng cho `[[mcp_clients]]`.
    assert!(config.mcp_client("cline").is_some());
    assert!(config.mcp_client("khong-co").is_none());
}

#[test]
fn mcp_client_without_identity_in_user_roles_is_rejected() {
    // Client có trong `[[mcp_clients]]` nhưng thiếu `mcp-client:<name>` trong
    // `user_roles` ⇒ sẽ là `no-access` và không thấy tool nào. Chặn ngay lúc nạp
    // cấu hình thay vì để người dùng tự tìm ra lúc client không thấy gì.
    let body = mcp_config("[mcp_server]\nenabled = true\n").replace(
        r#"user_roles = { "mcp-client:cline" = "monitor" }"#,
        "user_roles = {}",
    );
    let (_dir, path) = write_config(&body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("mcp-client:cline")),
        "phải báo thiếu identity, nhận {err:?}"
    );
}

#[test]
fn mcp_client_role_must_match_user_roles_and_exist() {
    // Role trong `[[mcp_clients]]` khác role trong `user_roles` ⇒ mâu thuẫn.
    // Thêm một role thật sự tồn tại (`ops`) rồi trỏ `[[mcp_clients]]` sang nó, để
    // nhánh kiểm "mâu thuẫn" chạy được (không vướng nhánh chặn role `admin`).
    let body = mcp_config("[mcp_server]\nenabled = true\n").replace(
        "[[roles]]\nname = \"admin\"",
        "[[roles]]\nname = \"ops\"\ntool_tags = [\"infra-read\"]\n\n[[roles]]\nname = \"admin\"",
    );
    let body = body.replace("role = \"monitor\"", "role = \"ops\"");
    let (_dir, path) = write_config(&body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("khác")),
        "phải báo mâu thuẫn role, nhận {err:?}"
    );

    // Role không tồn tại trong `[[roles]]`.
    let body = mcp_config("").replace("role = \"monitor\"\n", "role = \"khong-co\"\n");
    let (_dir, path) = write_config(&body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("không tồn tại")),
        "phải báo role không tồn tại, nhận {err:?}"
    );
}

#[test]
fn mcp_client_cannot_use_wildcard_admin_role() {
    // Role `admin` giữ tag `*` — cấp ở đường MCP chỉ gây hiểu nhầm, vì cổng expose
    // vẫn chặn mọi tool ghi/thực thi dù role có bao nhiêu quyền.
    let body = mcp_config("").replace("role = \"monitor\"", "role = \"admin\"");
    let (_dir, path) = write_config(&body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("read-only")),
        "phải chặn role admin cho MCP client, nhận {err:?}"
    );
}

#[test]
fn enabling_mcp_server_without_clients_is_rejected() {
    // Bật MCP server mà không có client ⇒ không ai xác thực được, chỉ là bề mặt mở.
    let (_dir, path) = write_config(&mcp_config("[mcp_server]\nenabled = true\n"));
    let mut config = Config::load(&path).unwrap();
    config.mcp_clients.clear();
    let err = config.validate().unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("mcp-token")),
        "phải nhắc chạy `auth mcp-token add`, nhận {err:?}"
    );
}

#[test]
fn mcp_http_bind_outside_loopback_requires_allow_remote() {
    // Cùng nguyên tắc với `[web]` (mục 15.7): lộ cổng phải là hành động tường minh.
    let body =
        mcp_config("[mcp_server]\nenabled = true\nhttp_enabled = true\nbind = \"0.0.0.0:7879\"\n");
    let (_dir, path) = write_config(&body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("allow_remote")),
        "phải bắt bật allow_remote, nhận {err:?}"
    );

    let body = body.replace(
        "bind = \"0.0.0.0:7879\"",
        "bind = \"0.0.0.0:7879\"\nallow_remote = true",
    );
    let (_dir, path) = write_config(&body);
    Config::load(&path).expect("bật allow_remote thì hợp lệ");
}

#[test]
fn mcp_client_name_must_be_clean() {
    // Tên client nằm trong identity `mcp-client:<name>` và trong path/header ⇒ chặn
    // ký tự lạ để khỏi tạo identity mơ hồ.
    for bad in ["co ten", "co/ten", ""] {
        let body = mcp_config("").replace("name = \"cline\"", &format!("name = \"{bad}\""));
        let (_dir, path) = write_config(&body);
        assert!(
            Config::load(&path).is_err(),
            "tên client `{bad}` phải bị từ chối"
        );
    }
}

#[test]
fn mcp_http_without_rate_limit_is_rejected() {
    // K24: bật transport HTTP (bề mặt có thể công khai) mà tắt trần tần suất thì cấu
    // hình phải chết lúc nạp, không âm thầm chạy không giới hạn.
    let body = mcp_config(
        "[mcp_server]\nenabled = true\nhttp_enabled = true\nrate_limit_per_minute = 0\n",
    );
    let (_dir, path) = write_config(&body);
    let err = Config::load(&path).unwrap_err();
    assert!(
        matches!(&err, ConfigError::Invalid(message) if message.contains("rate_limit_per_minute")),
        "phải chặn HTTP không giới hạn tần suất, nhận {err:?}"
    );
    // stdio (http_enabled = false) thì `0` vẫn hợp lệ: không có bề mặt từ xa.
    let (_dir, path) = write_config(&mcp_config(
        "[mcp_server]\nenabled = true\nrate_limit_per_minute = 0\n",
    ));
    Config::load(&path).expect("stdio thì không cần trần tần suất");
}

#[test]
fn mcp_rate_limit_defaults_and_overrides() {
    let (_dir, path) = write_config(&mcp_config("[mcp_server]\nenabled = true\n"));
    let config = Config::load(&path).unwrap();
    assert_eq!(config.mcp_server.rate_limit_per_minute, 120);
    assert_eq!(config.mcp_server.rate_limit_ip_multiplier, 5);

    let (_dir, path) = write_config(&mcp_config(
        "[mcp_server]\nenabled = true\nrate_limit_per_minute = 300\nrate_limit_ip_multiplier = 2\n",
    ));
    let config = Config::load(&path).unwrap();
    assert_eq!(config.mcp_server.rate_limit_per_minute, 300);
    assert_eq!(config.mcp_server.rate_limit_ip_multiplier, 2);
}

// ---------------------------------------------------------------------------
// `[[mcp_servers]]` — `call_timeout_seconds` + `inherit_env` (client side)
// ---------------------------------------------------------------------------

/// Cấu hình hợp lệ có đúng một `[[mcp_servers]]` với phần bổ sung do tham số truyền vào.
fn mcp_client_server(extra: &str) -> String {
    format!(
        r#"
[[mcp_servers]]
name = "example"
command = "/usr/local/bin/some-mcp-server"
{extra}
"#
    )
}

#[test]
fn mcp_server_inherits_defaults_for_new_fields() {
    // Cấu hình cũ (không có trường mới) phải nạp được: `None`/`rỗng` là hành vi
    // `env_clear()` nguyên bản, tức không hồi quy về "kế thừa toàn bộ host env".
    let (_dir, path) = write_config(&mcp_client_server(""));
    let config = Config::load(&path).expect("cấu hình cũ không cần trường mới");
    assert_eq!(config.mcp_servers.len(), 1);
    assert_eq!(config.mcp_servers[0].call_timeout_seconds, None);
    assert!(config.mcp_servers[0].inherit_env.is_empty());
}

#[test]
fn mcp_server_accepts_timeout_and_inherit_env() {
    let (_dir, path) = write_config(&mcp_client_server(
        "call_timeout_seconds = 300\ninherit_env = [\"PATH\", \"HOME\", \"DISPLAY\"]\n",
    ));
    let config = Config::load(&path).expect("giá trị hợp lệ phải nạp được");
    assert_eq!(config.mcp_servers[0].call_timeout_seconds, Some(300));
    assert_eq!(
        config.mcp_servers[0].inherit_env,
        vec!["PATH", "HOME", "DISPLAY"]
    );
}

#[test]
fn mcp_server_call_timeout_of_zero_is_rejected() {
    // 0 ⇒ `Duration::ZERO` ⇒ mọi tool call treo tức thì và trả lỗi. Model sẽ tưởng
    // server hỏng rồi thử lại, nên phải chết lúc nạp chứ không phải lúc chạy.
    let (_dir, path) = write_config(&mcp_client_server("call_timeout_seconds = 0\n"));
    let error = Config::load(&path).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Invalid(message) if message.contains("call_timeout_seconds")),
        "{error:?}"
    );
}

#[test]
fn mcp_server_call_timeout_above_ceiling_is_rejected() {
    let (_dir, path) = write_config(&mcp_client_server("call_timeout_seconds = 86400\n"));
    let error = Config::load(&path).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Invalid(message) if message.contains("call_timeout_seconds")),
        "{error:?}"
    );
}

#[test]
fn mcp_server_inherit_env_rejects_names_that_would_panic_command_env() {
    // `Command::env` panic khi tên rỗng / chứa '=' / NUL — phải thành ConfigError.
    for bad in [
        "\"\"",
        "\"A=B\"",
        "\"WITH NUL\\u0000\"",
        "\"HAS-DASH\"",
        "\"9LEAD\"",
    ] {
        let (_dir, path) = write_config(&mcp_client_server(&format!("inherit_env = [{bad}]\n")));
        let error = Config::load(&path).unwrap_err();
        assert!(
            matches!(&error, ConfigError::Invalid(message) if message.contains("inherit_env")),
            "inherit_env = [{bad}] phải bị chặn, nhận: {error:?}"
        );
    }
}

#[test]
fn mcp_server_duplicate_names_are_rejected() {
    // Tên server đi thẳng vào `mcp__<name>__<tool>`: trùng tên ⇒ tool bị đè âm thầm.
    // Kiểm tra này đã có sẵn ở `validate_web_and_channels`; giữ ở đây để chốt hành vi
    // sau khi thêm hai trường mới (rủi ro hồi quy: thêm field mà làm mất validate cũ).
    let (_dir, path) = write_config(&format!(
        "{}{}",
        mcp_client_server(""),
        "\n[[mcp_servers]]\nname = \"example\"\ncommand = \"/usr/local/bin/other\"\n"
    ));
    let error = Config::load(&path).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Invalid(message) if message.contains("trùng")),
        "{error:?}"
    );
}

#[test]
fn mcp_server_empty_command_is_rejected() {
    let (_dir, path) = write_config("\n[[mcp_servers]]\nname = \"broken\"\ncommand = \"  \"\n");
    let error = Config::load(&path).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Invalid(message) if message.contains("command")),
        "{error:?}"
    );
}

// ---------------------------------------------------------------------------
// M26 — `[browser]`
// ---------------------------------------------------------------------------

/// Nhóm `browser` là nhóm hợp lệ (nếu không, file mẫu không nạp được).
#[test]
fn browser_tool_group_is_known() {
    assert!(KNOWN_TOOL_GROUPS.contains(&BROWSER_TOOL_GROUP));
}

/// `[browser].enabled = false` thì KHÔNG validate whitelist — người dùng có thể
/// để sẵn mẫu đang viết rồi bật sau.
#[test]
fn disabled_browser_does_not_validate_origins() {
    let (_dir, path) = write_config("[browser]\nenabled = false\n");
    Config::load(&path).expect("browser tắt thì mẫu sai cũng không chặn");
}

/// Mẫu origin sai khi **bật** browser phải chặn ở tầng config.
#[test]
fn enabled_browser_rejects_malformed_origins() {
    for bad in [
        "dev.internal",             // thiếu scheme
        "https://dev.internal/x",   // có đường dẫn
        "https://u:p@dev.internal", // credential
        "https://*",                // quá rộng
        "https://dev.*.internal",   // wildcard sai vị trí
        "https://dev.internal:abc", // port không phải số
    ] {
        let (_dir, path) = write_config(&format!(
            "[browser]\nenabled = true\nallowed_origins = [\"{bad}\"]\n"
        ));
        let error = Config::load(&path).expect_err("phải chặn mẫu sai");
        assert!(
            matches!(&error, ConfigError::Invalid(m) if m.contains("allowed_origins")),
            "{bad}: {error:?}"
        );
    }
}

/// `max_image_bytes = 0` khiến mọi lần chụp thất bại ⇒ chặn.
#[test]
fn browser_rejects_zero_image_budget() {
    let (_dir, path) = write_config("[browser]\nenabled = true\nmax_image_bytes = 0\n");
    let error = Config::load(&path).expect_err("phải chặn");
    assert!(
        matches!(&error, ConfigError::Invalid(m) if m.contains("max_image_bytes")),
        "{error:?}"
    );
}

/// Mẫu origin hợp lệ phải nạp được.
#[test]
fn enabled_browser_accepts_valid_origins() {
    let (_dir, path) = write_config(
        "[browser]\nenabled = true\nexecutable_path = \"/usr/bin/google-chrome\"\n\
         allowed_origins = [\"http://localhost:3000\", \"https://*.dev.internal\"]\n",
    );
    let config = Config::load(&path).expect("cấu hình hợp lệ phải nạp được");
    assert!(config.browser.enabled);
    assert_eq!(config.browser.allowed_origins.len(), 2);
}
