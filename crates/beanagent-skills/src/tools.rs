//! Tool `load_skill` (Safe) và `create_skill` (Confirm).

use std::sync::Arc;

use beanagent_tools::{Tool, ToolCtx, ToolError, TypedTool};
use beanagent_types::Risk;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::loader::{SkillCatalog, SkillError};

/// Tải nội dung đầy đủ của một skill đã có trong catalog.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoadSkillParams {
    /// Tên skill chính xác như trong index `name: description`.
    pub name: String,
}

/// Tạo một skill mới trong thư mục skill của người dùng.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSkillParams {
    /// Tên kebab-case, không được chứa ký tự đường dẫn.
    pub name: String,
    /// Mô tả ngắn để model biết khi nào nạp skill, tối đa 300 ký tự.
    pub description: String,
    /// Nội dung hướng dẫn sau frontmatter của `SKILL.md`.
    pub body: String,
}

/// Trả về hai tool dùng chung một catalog.
#[must_use]
pub fn skill_tools(catalog: SkillCatalog) -> Vec<Arc<dyn Tool>> {
    vec![load_skill(catalog.clone()), create_skill(catalog)]
}

/// Tool an toàn để đọc hướng dẫn đầy đủ của skill.
#[must_use]
pub fn load_skill(catalog: SkillCatalog) -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
        "load_skill",
        Risk::Safe,
        move |_ctx: &ToolCtx, params: LoadSkillParams| {
            let catalog = catalog.clone();
            async move {
                let skill = catalog.get(&params.name).map_err(skill_error)?;
                Ok(format!(
                    "Skill: {}\nThư mục: {}\n\n{}",
                    skill.name,
                    skill.directory.display(),
                    skill.content
                ))
            }
        },
    ))
}

/// Tool có xác nhận để tạo skill mới, không bao giờ ghi đè.
#[must_use]
pub fn create_skill(catalog: SkillCatalog) -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
        "create_skill",
        Risk::Confirm,
        move |_ctx: &ToolCtx, params: CreateSkillParams| {
            let catalog = catalog.clone();
            async move {
                let params_name = params.name;
                let params_description = params.description;
                let params_body = params.body;
                let result = tokio::task::spawn_blocking(move || {
                    catalog.create(&params_name, &params_description, &params_body)
                })
                .await
                .map_err(|error| ToolError::Internal(error.to_string()))?;
                let skill = result.map_err(skill_error)?;
                Ok(format!(
                    "Đã tạo skill `{}` tại {}",
                    skill.name,
                    skill.directory.display()
                ))
            }
        },
    ))
}

fn skill_error(error: SkillError) -> ToolError {
    match error {
        SkillError::NotFound(name) => ToolError::NotFound(format!("skill `{name}`")),
        SkillError::AlreadyExists(name) => {
            ToolError::InvalidArgs(format!("skill `{name}` đã tồn tại; chọn tên khác"))
        }
        SkillError::InvalidName(message)
        | SkillError::InvalidDescription(message)
        | SkillError::InvalidFrontmatter(message) => ToolError::InvalidArgs(message),
        SkillError::Io(message) => ToolError::Io(message),
        SkillError::CatalogPoisoned => {
            ToolError::Internal("catalog skill không còn khả dụng".into())
        }
        SkillError::MissingField(field) => {
            ToolError::InvalidArgs(format!("thiếu trường frontmatter `{field}`"))
        }
    }
}
