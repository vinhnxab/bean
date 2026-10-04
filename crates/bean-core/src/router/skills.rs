//! Quản lý skill của Router: catalog, index progressive-disclosure, duyệt nháp.
//!
//! # Vì sao tách khỏi [`super::Router`]
//!
//! `skills` và `skills_index` là **một** khái niệm: index chỉ có nghĩa khi đọc từ
//! catalog, và phải được nạp lại đúng lúc một skill được kích hoạt. Trước đây chúng
//! là hai trường rời của `RouterInner` và logic "duyệt nháp ⇒ reload index" nằm lọt
//! trong `Router` — rất dễ xảy ra khi một đường kích hoạt skill mới quên reload.
//!
//! [`SkillCoordinator`] gom hai trường và **tự reload index trong cùng hàm** kích
//! hoạt, nên không còn đường nào duyệt skill mà quên đồng bộ index.
//!
//! Phân quyền `actor` **không** nằm ở đây: đó là kiểm tra danh tính của `Router`.

use std::sync::RwLock;

use bean_skills::{SkillCatalog, SkillDraftDecision, SkillError};

use super::RouterError;

/// Catalog skill + index `name: description` đưa vào system prompt.
#[derive(Debug)]
pub(super) struct SkillCoordinator {
    /// Catalog dùng chung; `None` khi không bật nhóm tool `skills`.
    catalog: Option<SkillCatalog>,
    /// Index đã dựng, phục vụ progressive disclosure (agents.md mục 9).
    index: RwLock<String>,
}

impl SkillCoordinator {
    /// Dựng từ phụ thuộc của `RouterDeps`.
    pub(super) fn new(catalog: Option<SkillCatalog>, index: String) -> Self {
        Self {
            catalog,
            index: RwLock::new(index),
        }
    }

    /// Bản index hiện tại cho system prompt.
    ///
    /// # Errors
    /// [`RouterError::StatePoisoned`] nếu khoá bị poison.
    pub(super) fn index(&self) -> Result<String, RouterError> {
        self.index
            .read()
            .map(|index| index.clone())
            .map_err(|_| RouterError::StatePoisoned)
    }

    /// Catalog đã cấu hình.
    ///
    /// # Errors
    /// [`RouterError::Skill`] khi nhóm tool `skills` chưa bật (không có catalog).
    pub(super) fn catalog(&self) -> Result<SkillCatalog, RouterError> {
        self.catalog
            .clone()
            .ok_or_else(|| SkillError::NotFound("skill catalog".into()).into())
    }

    /// Dựng lại index từ catalog — gọi sau **mọi** thao tác làm danh sách skill đổi.
    fn refresh_index(&self) -> Result<(), RouterError> {
        let index = self.catalog()?.index();
        *self.index.write().map_err(|_| RouterError::StatePoisoned)? = index;
        Ok(())
    }

    /// Duyệt và **kích hoạt** skill nháp, rồi đồng bộ index.
    ///
    /// # Errors
    /// [`RouterError::Skill`] khi id không tồn tại hoặc draft không hợp lệ.
    pub(super) async fn approve_draft(&self, id: &str) -> Result<SkillDraftDecision, RouterError> {
        let catalog = self.catalog()?;
        let draft_id = id.to_string();
        let decision = tokio::task::spawn_blocking(move || catalog.approve_draft(&draft_id))
            .await
            .map_err(|error| RouterError::Internal(error.to_string()))??;
        self.refresh_index()?;
        Ok(decision)
    }

    /// Bỏ skill nháp.
    ///
    /// Cố ý **không** reload index: bỏ nháp không làm danh sách skill đổi, nên reload
    /// ở đây chỉ tốn công ghi lại một index y hệt.
    ///
    /// # Errors
    /// [`RouterError::Skill`] khi id không tồn tại.
    pub(super) async fn reject_draft(&self, id: &str) -> Result<SkillDraftDecision, RouterError> {
        let catalog = self.catalog()?;
        let draft_id = id.to_string();
        tokio::task::spawn_blocking(move || catalog.reject_draft(&draft_id))
            .await
            .map_err(|error| RouterError::Internal(error.to_string()))?
            .map_err(Into::into)
    }
}
