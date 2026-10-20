//! Test persist `[[llm.providers]]` bằng `toml_edit` (Giai đoạn 3): round-trip
//! giữ nguyên định dạng các phần không đụng tới, và các ràng buộc validate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bean_types::config::{
    LlmProfile, LlmProviderKind, default_max_tokens, validate_profile_name, write_llm_providers,
};

fn profile(name: &str, model: &str) -> LlmProfile {
    LlmProfile {
        name: name.to_string(),
        provider: LlmProviderKind::OpenAiCompat,
        model: model.to_string(),
        allowed_models: vec![model.to_string()],
        api_key_env: String::new(),
        base_url: None,
        max_tokens: 4096,
    }
}

/// File cấu hình có comment + căn lề người dùng viết tay; phần `[agent]` và
/// comment phải được giữ nguyên byte sau khi CRUD thêm profile.
const SAMPLE_TOML: &str = r#"# Cấu hình Bean của tôi
# Không xoá comment này!

[agent]
timezone = "Asia/Ho_Chi_Minh"  # múi giờ hiển thị
max_steps = 25

[llm]
provider = "anthropic"

# Ollama chạy local
[[llm.providers]]
name = "ollama"
model = "qwen3:14b"

[[llm.providers]]
name = "paid"
model = "claude-opus-5"
api_key_env = "ANTHROPIC_API_KEY"
"#;

#[test]
fn round_trip_giu_nguyen_phan_khong_dung_toi() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, SAMPLE_TOML).unwrap();

    // Thêm một profile mới: ollama + paid phải còn nguyên, kèm profile mới.
    let paid = LlmProfile {
        api_key_env: "ANTHROPIC_API_KEY".to_string(),
        ..profile("paid", "claude-opus-5")
    };
    let providers = vec![
        profile("ollama", "qwen3:14b"),
        paid,
        profile("openrouter", "anthropic/claude-sonnet-5"),
    ];
    write_llm_providers(&path, &providers).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    // Comment và section ngoài [llm] giữ nguyên byte.
    assert!(out.contains("# Cấu hình Bean của tôi"));
    assert!(out.contains("# Không xoá comment này!"));
    assert!(out.contains("# múi giờ hiển thị"));
    assert!(out.contains("[agent]\ntimezone = \"Asia/Ho_Chi_Minh\"  # múi giờ hiển thị"));
    // Comment nằm TRONG mảng providers bị ghi đè toàn bộ — chấp nhận theo thiết kế.
    // Danh sách providers đúng thứ tự và đủ 3 entry.
    assert!(out.contains("[[llm.providers]]"));
    let name_positions: Vec<usize> = ["ollama", "paid", "openrouter"]
        .iter()
        .map(|name| out.find(&format!("name = \"{name}\"")).expect(name))
        .collect();
    assert!(
        name_positions[0] < name_positions[1] && name_positions[1] < name_positions[2],
        "thứ tự providers phải giữ nguyên khi thêm cuối: {out}"
    );
    // Profile `paid` còn api_key_env.
    assert!(out.contains("api_key_env = \"ANTHROPIC_API_KEY\""));
    // Parse lại được bằng Config::load (round-trip không phá cú pháp).
    let config = bean_types::Config::load(&path).unwrap();
    assert_eq!(config.llm.providers.len(), 3);
    assert_eq!(config.llm.providers[2].name, "openrouter");
}

#[test]
fn xoa_profile_giu_phan_con_lai_va_comment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, SAMPLE_TOML).unwrap();

    write_llm_providers(&path, &[profile("paid", "claude-opus-5")]).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains("name = \"paid\""));
    assert!(!out.contains("name = \"ollama\""));
    // Comment ngoài [llm] vẫn nguyên (comment mồ côi trước entry đã xoá không bắt xoá).
    assert!(out.contains("# Cấu hình Bean của tôi"));
    let config = bean_types::Config::load(&path).unwrap();
    assert_eq!(config.llm.providers.len(), 1);
}

#[test]
fn file_chua_co_llm_tao_moi_phan_llm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, "[agent]\nmax_steps = 25\n").unwrap();

    write_llm_providers(&path, &[profile("local", "m")]).unwrap();

    let config = bean_types::Config::load(&path).unwrap();
    assert_eq!(config.llm.providers.len(), 1);
    assert_eq!(config.llm.providers[0].name, "local");
}

#[test]
fn file_chua_ton_tai_tao_moi() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");

    write_llm_providers(&path, &[profile("local", "m")]).unwrap();

    let config = bean_types::Config::load(&path).unwrap();
    assert_eq!(config.llm.providers.len(), 1);
}

#[test]
fn danh_sach_rong_xoa_khoa_providers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, SAMPLE_TOML).unwrap();

    write_llm_providers(&path, &[]).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(!out.contains("[[llm.providers]]"));
    // [llm] vẫn còn (provider = "anthropic") — chỉ xoá khoá providers.
    assert!(out.contains("[llm]"));
    let config = bean_types::Config::load(&path).unwrap();
    assert!(config.llm.providers.is_empty());
}

#[test]
fn llm_inline_table_duoc_thay_bang_bang_day_du() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, "llm = { provider = \"anthropic\" }\n").unwrap();

    write_llm_providers(&path, &[profile("local", "m")]).unwrap();

    // Inline table không chèn được array-of-tables — phải thay bằng bảng đầy đủ
    // thay vì lỗi/panic.
    let config = bean_types::Config::load(&path).unwrap();
    assert_eq!(config.llm.providers.len(), 1);
}

#[test]
fn file_toml_hong_tra_loi_invalid_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bean.toml");
    std::fs::write(&path, "khong phai toml [").unwrap();

    let result = write_llm_providers(&path, &[profile("local", "m")]);
    let error = result.expect_err("file hỏng phải lỗi");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[test]
fn validate_profile_trong_luu_tru() {
    // model rỗng.
    let mut invalid = profile("ok", "");
    assert_eq!(invalid.validate_for_storage().unwrap_err().field, "model");
    // allowed_models chứa entry rỗng.
    invalid.model = "m".to_string();
    invalid.allowed_models = vec!["m".to_string(), "  ".to_string()];
    assert_eq!(
        invalid.validate_for_storage().unwrap_err().field,
        "allowed_models"
    );
    // api_key_env sai dạng.
    invalid.allowed_models = vec![];
    invalid.api_key_env = "1BAD-KEY".to_string();
    assert_eq!(
        invalid.validate_for_storage().unwrap_err().field,
        "api_key_env"
    );
    // base_url không phải http(s).
    invalid.api_key_env = String::new();
    invalid.base_url = Some("ftp://example.com".to_string());
    assert_eq!(
        invalid.validate_for_storage().unwrap_err().field,
        "base_url"
    );
    // base_url http(s) nhưng không có remainder.
    invalid.base_url = Some("https://".to_string());
    assert_eq!(
        invalid.validate_for_storage().unwrap_err().field,
        "base_url"
    );
    // max_tokens = 0.
    invalid.base_url = None;
    invalid.max_tokens = 0;
    assert_eq!(
        invalid.validate_for_storage().unwrap_err().field,
        "max_tokens"
    );
    // Hợp lệ: default max_tokens + mọi trường ổn.
    invalid.max_tokens = default_max_tokens();
    invalid.base_url = Some("http://127.0.0.1:9/v1".to_string());
    invalid.api_key_env = "BEAN_TEST_KEY".to_string();
    invalid.allowed_models = vec!["m1".to_string(), "m2".to_string()];
    assert!(invalid.validate_for_storage().is_ok());
}

#[test]
fn validate_ten_profile() {
    let existing = vec![profile("ollama", "m")];
    // Hợp lệ.
    assert!(validate_profile_name("openrouter-2", &existing).is_ok());
    // Rỗng / dành riêng / quá dài / ký tự lạ / trùng.
    assert!(validate_profile_name("", &existing).is_err());
    assert!(validate_profile_name("default", &existing).is_err());
    assert!(validate_profile_name(&"a".repeat(65), &existing).is_err());
    assert!(validate_profile_name("OpenRouter", &existing).is_err());
    assert!(validate_profile_name(" co.space ", &existing).is_err());
    assert!(validate_profile_name("ollama", &existing).is_err());
}
