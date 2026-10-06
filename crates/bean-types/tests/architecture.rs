#![allow(clippy::panic, clippy::expect_used)]
//! Chặn hồi quy kiến trúc ở cấu hình `bean.toml`.
//!
//! # Vì sao cần
//!
//! `config.rs` từng là **một** file 2372 dòng chứa mười sáu section của `bean.toml`,
//! bốn khối `impl Config` khổng lồ (nạp, truy vấn, validate, resolve secret) và mọi
//! validator của sản phẩm. Sửa một quy tắc validate phải mở file có cả secret và
//! RBAC.
//!
//! Cấu hình là **nguồn quyết định an toàn**: `validate()` là nơi chặn tổ hợp vô nghĩa
//! trước khi agent chạy, `resolve_*` là nơi bí mật không được lọt vào prompt. Sửa quy
//! tắc đó phải không cần lội 2000 dòng.
//!
//! Đây là "guardrail", không phải đo chất lượng: nó chỉ nói "file này đã lớn quá mức
//! dễ đọc", còn `clippy`/`fmt`/`cargo test` lo phần còn lại.

use std::path::{Path, PathBuf};

/// Trần số dòng cho **từng** file dưới `src/config/`.
/// Lớn nhất là `validate.rs` (~466) — dưới trần này.
const FILE_LINE_BUDGET: usize = 600;

/// Số file `.rs` tối thiểu trực tiếp dưới `src/config/`.
const MIN_CONFIG_MODULES: usize = 12;

/// Các module mà doc của `mod.rs` phải nhắc tới — bản đồ để người mới đọc.
const MOD_DOC_MUST_MENTION: &[&str] = &[
    "error.rs",
    "enums.rs",
    "agent.rs",
    "llm.rs",
    "tools.rs",
    "channels.rs",
    "integrations.rs",
    "products.rs",
    "load.rs",
    "access.rs",
    "validate.rs",
    "validate_integrations.rs",
    "secrets.rs",
];

/// Thư mục gốc `src/config`.
fn config_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config")
}

#[test]
fn no_config_file_exceeds_the_line_budget() {
    let dir = config_dir();
    let entries = std::fs::read_dir(&dir).expect("đọc `src/config`");
    let mut over = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let count = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("không đọc được `{name}`: {error}"))
            .lines()
            .count();
        if count >= FILE_LINE_BUDGET {
            over.push(format!("{name} ({count} dòng)"));
        }
    }
    assert!(
        over.is_empty(),
        "file vượt trần {FILE_LINE_BUDGET} dòng: {over:?}. Tách tiếp theo ranh giới section `bean.toml`."
    );
}

/// Không được gộp ngược về **một** file `config.rs` monolithic nữa.
#[test]
fn config_stays_split_and_keeps_its_modules() {
    let dir = config_dir();
    assert!(
        dir.is_dir(),
        "`src/config/` biến mất — cấu hình bị gộp lại thành một file?"
    );
    let entries = std::fs::read_dir(&dir).expect("đọc `src/config`");
    let mut files = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            files += 1;
        }
    }
    assert!(
        files >= MIN_CONFIG_MODULES,
        "`src/config/` chỉ còn {files} file (tối thiểu {MIN_CONFIG_MODULES}). Đừng gộp ngược về một file."
    );
}

/// Doc của `mod.rs` phải mô tả cây module.
#[test]
fn mod_doc_documents_the_module_tree() {
    let text = std::fs::read_to_string(config_dir().join("mod.rs")).expect("đọc `mod.rs`");
    for name in MOD_DOC_MUST_MENTION {
        assert!(
            text.contains(name),
            "`mod.rs` không nhắc tới `{name}` — bản đồ module đã lỗi thời."
        );
    }
}

/// Mọi mục công khai của `config` phải được `mod.rs` re-export: đường dẫn
/// `bean_types::config::X` là hợp đồng với **mọi crate còn lại** và với
/// `tests/config.rs`. Nếu re-export này bị cắt, chỉ `cargo build -p bean-types`
/// mới xanh — cả workspace sẽ đỏ.
#[test]
fn mod_reexports_the_public_config_api() {
    let text = std::fs::read_to_string(config_dir().join("mod.rs")).expect("đọc `mod.rs`");
    for name in [
        "ConfigError",
        "AgentConfig",
        "RoleConfig",
        "LlmConfig",
        "ToolsConfig",
        "WebConfig",
        "TelegramConfig",
        "McpServerConfig",
        "McpServerConfigSettings",
        "BrowserConfig",
        "QaRunner",
        "WebSearchConfig",
        "validate_scope_value",
    ] {
        assert!(
            text.contains(name),
            "`mod.rs` không re-export `{name}` — hợp đồng API với các crate khác bị phá."
        );
    }
}
