//! # BeanAgent-skills
//!
//! Loader và tool cho skill (agents.md mục 9).
//!
//! M6 tự parse frontmatter vì chỉ có hai scalar field `name`/`description`; không thêm
//! dependency YAML cho một định dạng nhỏ. `serde_yaml` không được dùng vì đã ngừng bảo trì.
#![forbid(unsafe_code)]

pub mod drafts;
pub mod loader;
pub mod tools;

pub use drafts::{
    NewSkillDraft, SkillDraft, SkillDraftDecision, SkillDraftKind, SkillDraftStatus,
    is_valid_draft_id,
};
pub use loader::{
    MAX_DESCRIPTION_CHARS, MAX_SKILL_CHARS, SKILL_FILE, Skill, SkillCatalog, SkillError,
};
pub use tools::{CreateSkillParams, LoadSkillParams, skill_tools};
