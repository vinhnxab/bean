//! Handler REST: skills va duyet skill nhap (M15).
//!
//! # Vì sao việc duyet nhap khong tự kích hoạt
//!
//! Skill do learning loop sinh ra phai do **nguoi dung duyet** moi bat (muc 17). Endpoint
//! nay chi chuyen quyet dinh cho Router, khong tu ghi file skill.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use bean_skills::{SkillDraft, SkillError};

use crate::api_types::{
    SkillDetail, SkillDraftDecisionRequest, SkillDraftDecisionResponse, SkillDraftDto,
    SkillDraftListResponse, SkillListResponse, SkillSummary,
};

use super::{ApiFailure, ApiResult, WebState, require_user};
pub(super) async fn list_skills(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SkillListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let skills = state
        .skills
        .list()
        .into_iter()
        .map(|skill| SkillSummary {
            name: skill.name,
            description: skill.description,
        })
        .collect();
    Ok(Json(SkillListResponse { skills }))
}

pub(super) async fn get_skill(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(name): Path<String>,
) -> ApiResult<Json<SkillDetail>> {
    let _ = require_user(&state, &jar).await?;
    let skill = state
        .skills
        .get(&name)
        .map_err(|_| ApiFailure::not_found())?;
    Ok(Json(SkillDetail {
        name: skill.name,
        description: skill.description,
        content: skill.content,
        directory: skill.directory.to_string_lossy().into_owned(),
    }))
}

pub(super) async fn list_skill_drafts(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SkillDraftListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let catalog = state.skills.clone();
    let drafts = tokio::task::spawn_blocking(move || catalog.list_drafts())
        .await
        .map_err(|_| ApiFailure::internal())?
        .map_err(skill_draft_api_error)?;
    Ok(Json(SkillDraftListResponse {
        drafts: drafts.iter().map(skill_draft_dto).collect(),
    }))
}

pub(super) async fn approve_skill_draft(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(_request): Json<SkillDraftDecisionRequest>,
) -> ApiResult<Json<SkillDraftDecisionResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let decision = state
        .router
        .approve_draft(&id, &user_id)
        .await
        .map_err(router_skill_error)?;
    Ok(Json(SkillDraftDecisionResponse {
        id: decision.id,
        status: decision.status.as_str().into(),
    }))
}

pub(super) async fn reject_skill_draft(
    State(state): State<WebState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Json(_request): Json<SkillDraftDecisionRequest>,
) -> ApiResult<Json<SkillDraftDecisionResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let decision = state
        .router
        .reject_draft(&id, &user_id)
        .await
        .map_err(router_skill_error)?;
    Ok(Json(SkillDraftDecisionResponse {
        id: decision.id,
        status: decision.status.as_str().into(),
    }))
}

fn skill_draft_dto(draft: &SkillDraft) -> SkillDraftDto {
    SkillDraftDto {
        id: draft.id.clone(),
        name: draft.name.clone(),
        kind: draft.kind.as_str().into(),
        description: draft.description.clone(),
        content: draft.content.clone(),
        reason: draft.reason.clone(),
        status: draft.status.as_str().into(),
        created_at: draft.created_at.clone(),
    }
}

fn router_skill_error(error: bean_core::RouterError) -> ApiFailure {
    match error {
        bean_core::RouterError::Skill(error) => skill_draft_api_error(error),
        bean_core::RouterError::Forbidden(_) => ApiFailure::forbidden(),
        _ => ApiFailure::internal(),
    }
}

fn skill_draft_api_error(error: SkillError) -> ApiFailure {
    match error {
        SkillError::DraftNotFound(_) | SkillError::NotFound(_) => ApiFailure::not_found(),
        SkillError::DraftAlreadyPending(_)
        | SkillError::DraftConflict(_)
        | SkillError::AlreadyExists(_) => ApiFailure::new(
            StatusCode::CONFLICT,
            "skill_draft_conflict",
            "skill hoặc bản sửa đã thay đổi",
        ),
        SkillError::InvalidName(_)
        | SkillError::InvalidDescription(_)
        | SkillError::InvalidFrontmatter(_)
        | SkillError::MissingField(_)
        | SkillError::MissingBody
        | SkillError::InvalidDraft(_) => ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_skill_draft",
            "skill nháp không hợp lệ",
        ),
        SkillError::Io(_) | SkillError::DraftRandom(_) | SkillError::CatalogPoisoned => {
            ApiFailure::internal()
        }
    }
}
