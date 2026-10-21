//! Ghi danh sách `[[llm.providers]]` vào `bean.toml` giữ nguyên định dạng (G3).
//!
//! CRUD profile từ UI cần sửa file cấu hình **mà không phá** phần người dùng
//! đã viết tay (comment, thứ tự section, căn lề). `toml_edit` chỉnh sửa theo
//! cấu trúc nên mọi phần không đụng tới được giữ nguyên byte. Toàn bộ mảng
//! `providers` được ghi đè mỗi lần — danh sách ngắn, và ghi đè toàn mảng đơn
//! giản hơn patch từng entry (không sót entry mồ côi khi xoá).
//!
//! Ghi atomic (tmp + rename) và giữ nguyên quyền của file gốc, như
//! [`super::LlmState::save`].

use std::path::Path;

use super::llm::{LlmProfile, default_max_tokens};

/// Ghi đè toàn bộ `[[llm.providers]]` của file cấu hình tại `path`.
///
/// File chưa tồn tại ⇒ tạo mới với đúng phần `[[llm.providers]]`. Danh sách
/// rỗng ⇒ xoá khoá `providers` khỏi `[llm]` (nếu có) thay vì ghi mảng rỗng.
///
/// Khi có ít nhất một profile chứa API key dán trực tiếp (`api_key`), file được
/// đặt quyền `0600` — chỉ owner đọc/ghi được vì key nằm plaintext trong file.
///
/// # Errors
/// [`std::io::Error`] khi file tồn tại nhưng không parse được TOML, hoặc không
/// ghi được file.
pub fn write_llm_providers(path: &Path, providers: &[LlmProfile]) -> std::io::Result<()> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut doc: toml_edit::DocumentMut = raw.parse().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("file cấu hình không parse được: {error}"),
        )
    })?;

    let llm_item = doc.entry("llm").or_insert(toml_edit::table());
    // `[llm]` viết dạng inline table (`llm = { … }`) thì không chèn được
    // array-of-tables — thay bằng bảng đầy đủ thay vì panic.
    if !llm_item.is_table() {
        *llm_item = toml_edit::table();
    }

    if providers.is_empty() {
        if let Some(table) = llm_item.as_table_like_mut() {
            table.remove("providers");
        }
    } else {
        let mut array = toml_edit::ArrayOfTables::new();
        for profile in providers {
            array.push(profile_to_table(profile));
        }
        llm_item["providers"] = toml_edit::Item::ArrayOfTables(array);
    }

    let out = doc.to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, out.as_bytes())?;
    // Giữ nguyên quyền của file gốc (thường là 0644); không có metadata cũ
    // (file mới) thì để umask quyết định.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    // Có key plaintext trong file ⇒ siết quyền còn 0600 (chỉ owner đọc/ghi).
    #[cfg(unix)]
    if providers.iter().any(|profile| profile.api_key.is_some()) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Dựng một bảng TOML từ profile, bỏ qua các trường bằng mặc định serde —
/// file ghi ra trông như người dùng tự viết tay.
fn profile_to_table(profile: &LlmProfile) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["name"] = toml_edit::value(profile.name.as_str());
    table["provider"] = toml_edit::value(profile.provider.as_str());
    table["model"] = toml_edit::value(profile.model.as_str());
    if !profile.allowed_models.is_empty() {
        let mut models = toml_edit::Array::new();
        for model in &profile.allowed_models {
            models.push(model.as_str());
        }
        table["allowed_models"] = toml_edit::value(models);
    }
    if !profile.api_key_env.is_empty() {
        table["api_key_env"] = toml_edit::value(profile.api_key_env.as_str());
    }
    if let Some(key) = &profile.api_key {
        table["api_key"] = toml_edit::value(key.as_str());
    }
    if let Some(base_url) = &profile.base_url {
        table["base_url"] = toml_edit::value(base_url.as_str());
    }
    if profile.max_tokens != default_max_tokens() {
        table["max_tokens"] = toml_edit::value(i64::from(profile.max_tokens));
    }
    table
}
