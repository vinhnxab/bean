//! Quyết định của người dùng: giải quyết confirm (allow/session/deny) và
//! duyệt/từ chối skill nháp (agents.md mục 9 + 10).

use super::*;

impl Router {
    /// Duyệt skill nháp sau khi actor đã xác thực ở lớp channel.
    ///
    /// Phân quyền `actor` là việc của `Router` (danh tính người gọi); phần catalog và
    /// đồng bộ index thuộc [`SkillCoordinator`].
    pub async fn approve_draft(
        &self,
        id: &str,
        actor: &str,
    ) -> Result<SkillDraftDecision, RouterError> {
        self.authorize_actor(actor)?;
        self.inner.skills.approve_draft(id).await
    }

    /// Bỏ skill nháp; không reload vì không có skill nào được kích hoạt.
    pub async fn reject_draft(
        &self,
        id: &str,
        actor: &str,
    ) -> Result<SkillDraftDecision, RouterError> {
        self.authorize_actor(actor)?;
        self.inner.skills.reject_draft(id).await
    }

    pub(super) fn authorize_actor(&self, actor: &str) -> Result<(), RouterError> {
        let config = read_lock(&self.inner.config)?;
        if config
            .agent
            .allowed_users
            .iter()
            .any(|allowed| allowed == actor)
        {
            Ok(())
        } else {
            Err(RouterError::Forbidden(actor.to_string()))
        }
    }

    /// Phản hồi hợp lệ đầu tiên thắng; response sau trả `ConfirmNotFound`.
    ///
    /// Quy tắc "ai thắng trước / đúng `actor` / chỉ `Confirm` mới được cho phép trong
    /// phiên" nằm ở [`ConfirmRegistry`]; ở đây chỉ trả quyết định về `RunIo` đang chờ
    /// và phát sự kiện ra ngoài.
    pub async fn resolve_confirm(
        &self,
        confirm_id: &str,
        decision: Decision,
        actor: &str,
    ) -> Result<(), RouterError> {
        let id = ConfirmId::new(confirm_id);
        let resolved = {
            let mut state = lock(&self.inner.state)?;
            ConfirmRegistry::resolve(&mut state, &id, decision, actor)?
        };
        resolved
            .sender
            .send(Confirmation {
                decision,
                actor: actor.to_string(),
            })
            .map_err(|_| RouterError::ConfirmNotFound)?;
        self.emit(RunEvent::ConfirmResolved {
            session_id: resolved.session_id,
            run_id: resolved.run_id,
            confirm_id: id,
            outcome: match decision {
                Decision::Allow | Decision::AllowInSession => ConfirmOutcome::Allowed,
                Decision::Deny => ConfirmOutcome::Denied,
            },
        });
        Ok(())
    }

    /// Đăng ký một yêu cầu xác nhận và phát `RunEvent::ConfirmRequest`.
    ///
    /// Nhận [`ConfirmRequest`] thay vì 7 đối số rời rạc: cấu trúc này đã dùng để gọi
    /// [`ConfirmRegistry::begin`], truyền xuyên suốt giúp không lệch định nghĩa và bỏ
    /// được `#[allow(clippy::too_many_arguments)]` trần.
    ///
    /// # Errors
    /// [`RouterError::Random`] khi OS không cấp được entropy cho `confirm_id`.
    pub(super) fn begin_confirm(
        &self,
        request: ConfirmRequest,
    ) -> Result<(ConfirmId, oneshot::Receiver<Confirmation>), RouterError> {
        let (receiver, announce) = {
            let mut state = lock(&self.inner.state)?;
            ConfirmRegistry::begin(&mut state, self.inner.options.confirm_timeout, request)?
        };
        // Phát đúng những gì đã lưu trong sổ (`timeout`, `role`) — không tính lại,
        // nếu lệch thì UI đếm ngược và HUB gắn agent sẽ sai.
        self.emit(RunEvent::ConfirmRequest {
            session_id: announce.session_id,
            run_id: announce.run_id,
            confirm_id: announce.confirm_id.clone(),
            prompt: announce.prompt,
            risk: announce.risk,
            allow_session_option: announce.allow_session_option,
            timeout_seconds: announce.timeout_seconds,
            role: announce.role,
        });
        Ok((announce.confirm_id, receiver))
    }

    pub(super) fn expire_confirm(&self, confirm_id: &ConfirmId, outcome: ConfirmOutcome) -> bool {
        let removed = lock(&self.inner.state)
            .ok()
            .and_then(|mut state| ConfirmRegistry::expire(&mut state, confirm_id));
        let Some((session_id, run_id)) = removed else {
            return false;
        };
        self.emit(RunEvent::ConfirmResolved {
            session_id,
            run_id,
            confirm_id: confirm_id.clone(),
            outcome,
        });
        true
    }
}
