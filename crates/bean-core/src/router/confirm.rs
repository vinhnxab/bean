//! Vòng đời yêu cầu xác nhận (agents.md mục 10, 15.3).
//!
//! # Vì sao tách khỏi [`super::Router`]
//!
//! Xác nhận là một **máy trạng thái riêng**: đăng ký → chờ → được duyệt / từ chối /
//! hết hạn → dọn. Nó có bất biến riêng, không thuộc về điều phối run:
//!
//! * phản hồi hợp lệ **đầu tiên thắng**, phản hồi sau trả `ConfirmNotFound`;
//! * chỉ đúng `actor` mới được phản hồi;
//! * chỉ `Confirm` mới được `AllowInSession`;
//! * hết hạn (hoặc run kết thúc) thì confirm bị gỡ khỏi sổ.
//!
//! Gom vào [`ConfirmRegistry`] để các quy tắc này nằm đúng chỗ; `Router` chỉ còn
//! phần phát sự kiện ra ngoài.
//!
//! # Về khoá
//!
//! Registry **không tự khoá**: `RouterState` vẫn nằm sau đúng một `Mutex` của
//! `Router`, và mỗi hàm ở đây nhận `&mut RouterState`. Nhờ vậy không phát sinh thêm
//! thứ tự khoá nào — một điểm quan trọng vì `finish` cũng đụng `state.confirms`.

use std::time::Duration;

use bean_types::{ConfirmId, Risk, RunId, SessionId};
use tokio::sync::oneshot;

use crate::run_io::Decision;

use super::new_confirm_id;
use super::{Confirmation, PendingConfirm, PendingConfirmInfo, RouterError, RouterState};

/// Yêu cầu xác nhận do `RunIo` phát ra.
///
/// Gom tham số thành struct để `begin` không phải nhận 9 đối số rời rạc (tránh
/// `clippy::too_many_arguments`, và tránl lỡ hoán đổi `actor` với `prompt`).
pub(super) struct ConfirmRequest {
    /// Session sở hữu run đang chờ duyệt.
    pub session_id: SessionId,
    /// Run đang chờ duyệt.
    pub run_id: RunId,
    /// Mức rủi ro của hành động.
    pub risk: Risk,
    /// Nội dung hành động hiển thị cho người duyệt.
    pub prompt: String,
    /// Có được hiện tuỳ chọn "cho phép trong phiên" không.
    pub allow_session_option: bool,
    /// Người được phép phản hồi.
    pub actor: String,
    /// Thời gian chờ do adapter xin; còn bị chặn trần bởi `max_timeout`.
    pub requested_timeout: Duration,
}

/// Kết quả phân giải một confirm hợp lệ (đã gỡ khỏi sổ).
pub(super) struct Resolved {
    /// Session sở hữu run đang chờ duyệt.
    pub session_id: SessionId,
    /// Run đang chờ duyệt.
    pub run_id: RunId,
    /// Kênh trả quyết định về `RunIo` đang chờ.
    pub sender: oneshot::Sender<Confirmation>,
}

/// Thông tin phát `RunEvent::ConfirmRequest` **sau khi** confirm đã đăng ký vào sổ.
///
/// Tách riêng để `Router` không phải tính lại `timeout`/tra lại `role` — hai thứ
/// phải khớp *chính xác* với bản ghi đã lưu, nếu lệch thì UI đếm ngược sai.
pub(super) struct Announce {
    /// Session sở hữu run.
    pub session_id: SessionId,
    /// Run đang chờ duyệt.
    pub run_id: RunId,
    /// Id confirm vừa sinh.
    pub confirm_id: ConfirmId,
    /// Nội dung hành động.
    pub prompt: String,
    /// Mức rủi ro.
    pub risk: Risk,
    /// Có cho phép "cho phép trong phiên" hay không.
    pub allow_session_option: bool,
    /// Thời gian chờ đã bị chặn trần, tính bằng giây.
    pub timeout_seconds: u32,
    /// Role của agent đang chờ duyệt (HUB cần nó để gắn về đúng agent).
    pub role: Option<String>,
}

/// Toàn bộ quy tắc của máy trạng thái xác nhận.
pub(super) struct ConfirmRegistry;

impl ConfirmRegistry {
    /// Đăng ký một yêu cầu xác nhận mới và trả kênh chờ quyết định.
    ///
    /// # Errors
    /// [`RouterError::Random`] khi OS không cấp được entropy cho `confirm_id`.
    pub(super) fn begin(
        state: &mut RouterState,
        max_timeout: Duration,
        request: ConfirmRequest,
    ) -> Result<(oneshot::Receiver<Confirmation>, Announce), RouterError> {
        let ConfirmRequest {
            session_id,
            run_id,
            risk,
            prompt,
            allow_session_option,
            actor,
            requested_timeout,
        } = request;
        let confirm_id = new_confirm_id()?;
        let (sender, receiver) = oneshot::channel();
        let timeout = requested_timeout.min(max_timeout);
        let timeout_seconds = u32::try_from(timeout.as_secs()).unwrap_or(u32::MAX);
        // Tra role của chính run đang chờ duyệt (không phải role resolve lại từ
        // `actor`): `actor` là người sẽ *trả lời*, còn HUB cần biết *agent nào* đang
        // chờ. Khi run đã kết thúc, `active` không còn ⇒ `None`, đúng như lúc đó
        // confirm sẽ bị dọn theo `finish`.
        let role = state.queue.role_of(&run_id);
        state.confirms.insert(
            confirm_id.clone(),
            PendingConfirm {
                run_id: run_id.clone(),
                session_id,
                actor,
                prompt: prompt.clone(),
                risk,
                allow_session_option,
                timeout_seconds,
                role: role.clone(),
                sender,
            },
        );
        let announce = Announce {
            session_id,
            run_id,
            confirm_id,
            prompt,
            risk,
            allow_session_option,
            timeout_seconds,
            role,
        };
        Ok((receiver, announce))
    }
    /// Phản hồi hợp lệ **đầu tiên** thắng.
    ///
    /// # Errors
    /// * [`RouterError::ConfirmNotFound`] — không có confirm nào với id này (đã hết
    ///   hạn, hoặc đã được phản hồi trước đó).
    /// * [`RouterError::ConfirmForbidden`] — `actor` không khớp, hoặc xin
    ///   `AllowInSession` cho hành động không có tuỳ chọn đó (ví dụ `Dangerous`).
    ///
    /// Cả hai nhánh lỗi đều **trả confirm về sổ**: yêu cầu đang chờ không bị nuốt mất.
    pub(super) fn resolve(
        state: &mut RouterState,
        id: &ConfirmId,
        decision: Decision,
        actor: &str,
    ) -> Result<Resolved, RouterError> {
        let pending = state
            .confirms
            .remove(id)
            .ok_or(RouterError::ConfirmNotFound)?;
        if pending.actor != actor {
            state.confirms.insert(id.clone(), pending);
            return Err(RouterError::ConfirmForbidden);
        }
        if matches!(decision, Decision::AllowInSession) && !pending.allow_session_option {
            state.confirms.insert(id.clone(), pending);
            return Err(RouterError::ConfirmForbidden);
        }
        Ok(Resolved {
            session_id: pending.session_id,
            run_id: pending.run_id,
            sender: pending.sender,
        })
    }

    /// Gỡ confirm do kết thúc run hoặc hết hạn.
    ///
    /// `None` ⇒ đã bị ai đó xử lý trước đó (đúng nghĩa "ai thắng trước thì thắng").
    pub(super) fn expire(state: &mut RouterState, id: &ConfirmId) -> Option<(SessionId, RunId)> {
        state
            .confirms
            .remove(id)
            .map(|pending| (pending.session_id, pending.run_id))
    }

    /// Gỡ mọi confirm thuộc một run vừa kết thúc.
    ///
    /// Trả về `(confirm_id, session_id, run_id)` để `Router` phát `ConfirmResolved`
    /// với outcome `Denied` — nếu thiếu bước này, UI sẽ đếm ngược tới hết hạn một thẻ
    /// duyệt đã vô nghĩa.
    pub(super) fn drop_for_run(
        state: &mut RouterState,
        run_id: &RunId,
    ) -> Vec<(ConfirmId, SessionId, RunId)> {
        let stale: Vec<_> = state
            .confirms
            .iter()
            .filter(|(_, pending)| pending.run_id == *run_id)
            .map(|(id, pending)| (id.clone(), pending.session_id, pending.run_id.clone()))
            .collect();
        for (id, _, _) in &stale {
            state.confirms.remove(id);
        }
        stale
    }

    /// Danh sách confirm đang chờ — nguồn cho `Sync` sau khi WebSocket nối lại.
    pub(super) fn pending(state: &RouterState) -> Vec<PendingConfirmInfo> {
        state
            .confirms
            .iter()
            .map(|(id, pending)| PendingConfirmInfo {
                confirm_id: id.clone(),
                session_id: pending.session_id,
                run_id: pending.run_id.clone(),
                prompt: pending.prompt.clone(),
                risk: pending.risk,
                allow_session_option: pending.allow_session_option,
                timeout_seconds: pending.timeout_seconds,
                role: pending.role.clone(),
            })
            .collect()
    }
}
