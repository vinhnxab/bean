//! Slash command của lõi (agents.md mục 10).
//!
//! # Vì sao tách phần *phân tích* ra khỏi phần *thực thi*
//!
//! Trước đây `Router::handle_command` vừa cắt chuỗi, vừa kiểm tra số tham số, vừa
//! `match` trên tên lệnh, vừa chạy tác dụng. Ba việc đó lẫn vào nhau khiến cây
//! nhánh phải lặp lại điều kiện "tham số có rỗng không" ở từng lệnh, và **không kiểm
//! thử được** phần cú pháp mà không phải dựng cả `Router`.
//!
//! Ở đây chỉ còn cú pháp: [`parse`] biến chuỗi thô thành [`Command`] (hoặc
//! [`ParseError`]). Phần thực thi thuộc về `Router` và `match` trên enum — bao giờ
//! cũng không có nhánh thừa, vì "không có tham số" đã được biến thành biến thể riêng.

use super::RouterError;

/// Một slash command đã được phân tích.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Command<'a> {
    /// `/new` — lưu trữ phiên cũ và tạo phiên mới.
    New,
    /// `/stop` — dừng run đang chạy của phiên.
    Stop,
    /// `/model` — xem model hiện tại và danh sách được phép.
    ShowModel,
    /// `/model <tên>` — đổi model (chỉ trong `llm.allowed_models`).
    SetModel(&'a str),
    /// `/skills` — liệt kê skill đang có.
    Skills,
    /// `/memory` — thiếu truy vấn.
    MemoryUsage,
    /// `/memory <truy vấn>` — tìm trong bộ nhớ dài hạn.
    MemorySearch(&'a str),
    /// `/tasks` — liệt kê tác vụ định kỳ.
    Tasks,
    /// `/approve <id>` — duyệt skill nháp (`id` có thể rỗng ⇒ in cách dùng).
    ApproveDraft(&'a str),
    /// `/reject <id>` — bỏ skill nháp (`id` có thể rỗng ⇒ in cách dùng).
    RejectDraft(&'a str),
}

/// Lỗi phân tích slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParseError {
    /// Người dùng gõ thừa tham số (lệnh này chỉ nhận tối đa 1).
    TooManyArguments,
    /// Không có lệnh nào khớp.
    Unknown(String),
}

impl ParseError {
    /// Mã ổn định cho UI.
    pub(super) const fn code(&self) -> &'static str {
        match self {
            Self::TooManyArguments => "invalid_arguments",
            Self::Unknown(_) => "unknown_command",
        }
    }

    /// Thông báo cho người dùng — dùng chung `Display` với [`RouterError`] để
    /// không lệch nội dung so với đường gọi cũ.
    pub(super) fn message(&self) -> String {
        match self {
            Self::TooManyArguments => "Quá nhiều tham số.".to_string(),
            Self::Unknown(name) => RouterError::UnknownCommand(name.clone()).to_string(),
        }
    }
}

/// Cắt chuỗi lệnh thô thành [`Command`].
///
/// Mọi lệnh nhận **tối đa một** tham số; quá thì trả [`ParseError::TooManyArguments`].
/// Không lệnh nào khớp thì trả [`ParseError::Unknown`].
///
/// # Errors
/// [`ParseError`] như mô tả trên.
pub(super) fn parse(input: &str) -> Result<Command<'_>, ParseError> {
    let mut parts = input.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let argument = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        return Err(ParseError::TooManyArguments);
    }
    Ok(match name {
        "/new" => Command::New,
        "/stop" => Command::Stop,
        "/model" if argument.is_empty() => Command::ShowModel,
        "/model" => Command::SetModel(argument),
        "/skills" => Command::Skills,
        "/memory" if argument.is_empty() => Command::MemoryUsage,
        "/memory" => Command::MemorySearch(argument),
        "/tasks" => Command::Tasks,
        "/approve" => Command::ApproveDraft(argument),
        "/reject" => Command::RejectDraft(argument),
        other => return Err(ParseError::Unknown(other.to_string())),
    })
}

use bean_types::{RunEvent, RunId, SessionId};

use super::{Incoming, Router, lock, read_lock, router_error_code, write_lock};

impl Router {
    pub(super) async fn handle_command(
        &self,
        incoming: &Incoming,
        session: SessionId,
        run_id: RunId,
        command: &str,
    ) -> Result<(), RouterError> {
        let parsed = match parse(command) {
            Ok(parsed) => parsed,
            Err(error) => {
                return self.command_error(session, run_id, error.code(), &error.message());
            }
        };
        let result: Result<String, RouterError> = match parsed {
            Command::New => {
                if self.session_busy(session) {
                    Err(RouterError::SessionBusy)
                } else {
                    self.inner.store.archive_session(session).await?;
                    lock(&self.inner.state)?.policies.remove(&session);
                    let new_session = self
                        .inner
                        .store
                        .ensure_session_for_user(
                            &incoming.channel,
                            &incoming.chat_id,
                            &incoming.user_id,
                            "",
                        )
                        .await?;
                    Ok(format!("Đã tạo phiên mới: {new_session}"))
                }
            }
            Command::Stop => {
                self.cancel_session(session).await;
                Ok("Đã yêu cầu dừng run hiện tại.".into())
            }
            Command::ShowModel => {
                let config = read_lock(&self.inner.config)?;
                Ok(format!(
                    "Model hiện tại: {}. Cho phép: {}",
                    config.llm.model,
                    config.llm.effective_allowed_models().join(", ")
                ))
            }
            Command::SetModel(argument) => {
                let mut config = write_lock(&self.inner.config)?;
                if config
                    .llm
                    .effective_allowed_models()
                    .iter()
                    .any(|model| model == argument)
                {
                    config.llm.model = argument.to_string();
                    Ok(format!("Model đổi thành: {argument}"))
                } else {
                    Err(RouterError::ModelNotAllowed(argument.to_string()))
                }
            }
            Command::Skills => {
                let index = self.inner.skills.index()?;
                Ok(if index.trim().is_empty() {
                    "Chưa nạp skill.".into()
                } else {
                    index
                })
            }
            Command::MemoryUsage => Ok("Dùng: /memory <truy vấn>".into()),
            Command::MemorySearch(query) => {
                let hits = self.inner.store.memory_search(query).await?;
                Ok(if hits.is_empty() {
                    "Không tìm thấy ghi nhớ.".into()
                } else {
                    hits.into_iter()
                        .map(|hit| format!("- {}", hit.text))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
            Command::Tasks => Ok("Chưa có tác vụ định kỳ.".into()),
            Command::ApproveDraft("") => Ok("Dùng: /approve <id>".into()),
            Command::RejectDraft("") => Ok("Dùng: /reject <id>".into()),
            Command::ApproveDraft(id) => self
                .approve_draft(id, &incoming.user_id)
                .await
                .map(|decision| format!("Đã duyệt và kích hoạt skill `{}`.", decision.name)),
            Command::RejectDraft(id) => self
                .reject_draft(id, &incoming.user_id)
                .await
                .map(|decision| format!("Đã bỏ skill nháp `{}`.", decision.name)),
        };
        match result {
            Ok(text) => {
                self.emit(RunEvent::Final {
                    session_id: session,
                    run_id,
                    text,
                    message_id: None,
                });
                Ok(())
            }
            Err(error) => {
                self.emit(RunEvent::Error {
                    session_id: session,
                    run_id,
                    code: router_error_code(&error).into(),
                    message: error.to_string(),
                });
                Ok(())
            }
        }
    }

    fn command_error(
        &self,
        session: SessionId,
        run_id: RunId,
        code: &str,
        message: &str,
    ) -> Result<(), RouterError> {
        self.emit(RunEvent::Error {
            session_id: session,
            run_id,
            code: code.into(),
            message: message.into(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Command, ParseError, parse};

    #[test]
    fn parses_commands_without_argument() {
        assert_eq!(parse("/new"), Ok(Command::New));
        assert_eq!(parse("/stop"), Ok(Command::Stop));
        assert_eq!(parse("/skills"), Ok(Command::Skills));
        assert_eq!(parse("/tasks"), Ok(Command::Tasks));
    }

    #[test]
    fn model_switches_between_query_and_set() {
        assert_eq!(parse("/model"), Ok(Command::ShowModel));
        assert_eq!(parse("/model  claude "), Ok(Command::SetModel("claude")));
    }

    #[test]
    fn memory_usage_is_its_own_variant_not_an_empty_query() {
        assert_eq!(parse("/memory"), Ok(Command::MemoryUsage));
        assert_eq!(parse("/memory  sơmi "), Ok(Command::MemorySearch("sơmi")));
    }

    #[test]
    fn draft_commands_keep_their_argument_even_when_empty() {
        assert_eq!(parse("/approve"), Ok(Command::ApproveDraft("")));
        assert_eq!(parse("/approve  d1 "), Ok(Command::ApproveDraft("d1")));
        assert_eq!(parse("/reject"), Ok(Command::RejectDraft("")));
        assert_eq!(parse("/reject  d2 "), Ok(Command::RejectDraft("d2")));
    }

    #[test]
    fn rejects_a_third_token() {
        assert_eq!(parse("/memory a b"), Err(ParseError::TooManyArguments));
        assert_eq!(parse("/model a b"), Err(ParseError::TooManyArguments));
    }

    /// Hành vi **có sẵn từ trước**, giữ nguyên có chủ ý: lệnh không dùng tham số
    /// vẫn nuốt token thừa thay vì báo lỗi (`/new x` chạy `/new` và bỏ qua `x`).
    /// Test này chốt hành vi để một đổi nhầm sẽ bị bắt.
    #[test]
    fn commands_without_argument_swallow_extra_token() {
        assert_eq!(parse("/new x"), Ok(Command::New));
        assert_eq!(parse("/stop x"), Ok(Command::Stop));
    }

    #[test]
    fn rejects_unknown_command_and_keeps_its_name() {
        assert_eq!(
            parse("/nope"),
            Err(ParseError::Unknown("/nope".to_string()))
        );
        assert_eq!(parse("").unwrap_err().code(), "unknown_command");
    }
}

// ---------------------------------------------------------------------------
// Thực thi slash command
//
// [`parse`] ở trên chỉ lo cú pháp; phần *tác dụng* nằm ở đây. Cùng module để một
// đầu vào slash command nằm trọn trong một chỗ, và `router.rs` không phải mang
// thêm một `match` dài.
// ---------------------------------------------------------------------------
