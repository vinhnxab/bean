//! Muc tieu theo doi va phan tich callback cua Telegram.
//!
//! # Vai tro
//!
//! Telegram gui **run_id** nhung **khong gui session_id**. Vay vua phai giu bang
//! `RunTarget` de khi su kien `RunEvent` ve, biet gui tin cho chat nao. Sau mot run,
//! mot callback confirm, mot skill nhap — ca deu la mot cap (run_id, chat_id) rieng.
//!
//! # Vì sao tach rieng
//!
//! Day la **bang tra cuoc** cua channel, doc lap hoan toan voi logic gui tin. No cung
//! la noi rat de sinh loi race: mot callback den sau khi run da ket thuc se khong co
//! target de gui, va xu ly sai se lam UI cua nguoi dung hien o sai noi.

use bean_core::Decision;
use bean_skills::is_valid_draft_id;
use bean_types::{ConfirmOutcome, RunEvent, RunId};

#[derive(Clone, Debug)]
pub(super) struct RunTarget {
    pub(super) chat_id: i64,
    pub(super) user_id: String,
}

#[derive(Clone, Debug)]
pub(super) struct DraftTarget {
    pub(super) chat_id: i64,
    pub(super) message_id: i32,
    pub(super) user_id: String,
}

#[derive(Clone, Debug)]
pub(super) struct ConfirmTarget {
    pub(super) chat_id: i64,
    pub(super) message_id: i32,
    pub(super) user_id: String,
    pub(super) allow_session: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CallbackAction {
    Confirm(Decision),
    ApproveDraft,
    RejectDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedCallback {
    pub(super) action: CallbackAction,
    pub(super) id: String,
}

pub(super) fn parse_callback(data: &str) -> Option<ParsedCallback> {
    if data.len() > 64 {
        return None;
    }
    let (action, id) = data.rsplit_once(':')?;
    if is_valid_confirm_id(id) {
        let decision = match action {
            "a" => Decision::Allow,
            "s" => Decision::AllowInSession,
            "d" => Decision::Deny,
            _ => return None,
        };
        return Some(ParsedCallback {
            action: CallbackAction::Confirm(decision),
            id: id.to_owned(),
        });
    }
    if is_valid_draft_id(id) {
        let action = match action {
            "skill:a" => CallbackAction::ApproveDraft,
            "skill:r" => CallbackAction::RejectDraft,
            _ => return None,
        };
        return Some(ParsedCallback {
            action,
            id: id.to_owned(),
        });
    }
    None
}

pub(super) fn is_valid_confirm_id(value: &str) -> bool {
    value.len() == 40
        && value.starts_with("confirm_")
        && value
            .as_bytes()
            .get(8..)
            .is_some_and(|suffix| suffix.len() == 32 && suffix.iter().all(u8::is_ascii_hexdigit))
}

pub(super) fn event_run_id(event: &RunEvent) -> &RunId {
    match event {
        RunEvent::Queued { run_id, .. }
        | RunEvent::Text { run_id, .. }
        | RunEvent::TextDelta { run_id, .. }
        | RunEvent::ToolStart { run_id, .. }
        | RunEvent::ToolEnd { run_id, .. }
        | RunEvent::ConfirmRequest { run_id, .. }
        | RunEvent::ConfirmResolved { run_id, .. }
        | RunEvent::Final { run_id, .. }
        | RunEvent::Error { run_id, .. } => run_id,
    }
}

pub(super) fn resolved_text(outcome: ConfirmOutcome) -> &'static str {
    match outcome {
        ConfirmOutcome::Allowed => "Đã cho phép",
        ConfirmOutcome::Denied => "Từ chối",
        ConfirmOutcome::Expired => "Hết hạn",
    }
}
