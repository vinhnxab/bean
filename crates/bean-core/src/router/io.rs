//! Kênh I/O của một run, hiện thực [`RunIo`] (agents.md mục 6, 10).
//!
//! # Vì sao tách khỏi [`super::Router`]
//!
//! `RouterIo` là **adapter phía kênh**: biến tiến trình của agent loop thành
//! `RunEvent` và biến yêu cầu xác nhận thành `oneshot`. Nó là cầu nối, không phải
//! logic của Router — để chung file làm `router.rs` dài thêm mà không thuộc về nó.
//!
//! Giữ `Weak<RouterInner>` (không phải `Arc`) để một `RunIo` bị bỏ rơi không giữ
//! Router sống sau khi tiến trình tắt.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use bean_types::{ConfirmOutcome, Risk, RunEvent, RunId, SessionId};
use tokio_util::sync::CancellationToken;

use super::{Router, RouterInner, preview, remember_actor};
use crate::run_io::{Decision, RunIo};

pub(super) struct RouterIo {
    inner: Weak<RouterInner>,
    session_id: SessionId,
    run_id: RunId,
    user_id: String,
    cancel: CancellationToken,
    background_allowed_tools: Option<Arc<HashSet<String>>>,
    last_actor: Mutex<Option<String>>,
}

#[async_trait::async_trait]
impl RunIo for RouterIo {
    fn on_text(&self, text: &str) {
        self.with_router(|router| {
            router.emit(RunEvent::Text {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                text: text.to_string(),
            });
        });
    }

    fn on_text_delta(&self, text: &str, index: u32, reset: bool) {
        self.with_router(|router| {
            router.emit(RunEvent::TextDelta {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                text: text.to_string(),
                index,
                reset,
            });
        });
    }

    fn on_tool_start(&self, id: &str, tool: &str, risk: Risk, summary: &str, args: &str) {
        self.with_router(|router| {
            router.emit(RunEvent::ToolStart {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                id: id.to_string(),
                tool: tool.to_string(),
                summary: preview(summary),
                args_preview: preview(args),
                risk,
            });
        });
    }

    fn on_tool_end(&self, id: &str, tool: &str, ok: bool, output: &str) {
        self.with_router(|router| {
            router.emit(RunEvent::ToolEnd {
                session_id: self.session_id,
                run_id: self.run_id.clone(),
                id: id.to_string(),
                tool: tool.to_string(),
                ok,
                output_preview: preview(output),
            });
        });
    }

    async fn confirm(
        &self,
        _id: &str,
        _tool: &str,
        risk: Risk,
        prompt: &str,
        allow_in_session: bool,
        timeout: Duration,
    ) -> Option<Decision> {
        if self.background_allowed_tools.is_some() {
            return Some(if self.background_tool_allowed(_tool) {
                Decision::Allow
            } else {
                Decision::Deny
            });
        }
        let inner = self.inner.upgrade()?;
        let router = Router { inner };
        let (confirm_id, receiver) = router
            .begin_confirm(
                self.session_id,
                &self.run_id,
                risk,
                prompt,
                allow_in_session,
                &self.user_id,
                timeout,
            )
            .ok()?;
        let effective_timeout = timeout.min(router.inner.options.confirm_timeout);
        let mut receiver = receiver;
        let decision = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => {
                if router.expire_confirm(&confirm_id, ConfirmOutcome::Denied) {
                    None
                } else {
                    receiver.await.ok().map(|confirmation| {
                        remember_actor(&self.last_actor, confirmation.actor);
                        confirmation.decision
                    })
                }
            }
            response = &mut receiver => response.ok().map(|confirmation| {
                remember_actor(&self.last_actor, confirmation.actor);
                confirmation.decision
            }),
            _ = tokio::time::sleep(effective_timeout) => {
                if router.expire_confirm(&confirm_id, ConfirmOutcome::Expired) {
                    None
                } else {
                    receiver.await.ok().map(|confirmation| {
                        remember_actor(&self.last_actor, confirmation.actor);
                        confirmation.decision
                    })
                }
            }
        };
        decision
    }

    fn decision_actor(&self) -> Option<String> {
        if self.background_allowed_tools.is_some() {
            return Some("scheduler".into());
        }
        self.last_actor.lock().ok().and_then(|actor| actor.clone())
    }

    fn is_background(&self) -> bool {
        self.background_allowed_tools.is_some()
    }

    fn background_tool_allowed(&self, tool: &str) -> bool {
        self.background_allowed_tools
            .as_ref()
            .is_some_and(|tools| tools.contains(tool))
    }

    fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

impl RouterIo {
    fn with_router(&self, action: impl FnOnce(&Router)) {
        if let Some(inner) = self.inner.upgrade() {
            action(&Router { inner });
        }
    }
}

impl RouterIo {
    /// Dựng kênh I/O cho một run. Chỉ [`Router`] gọi — trường của `RouterIo` để
    /// private ngoài module này, nên việc dựng đi qua đây giữ đóng gói.
    pub(super) fn new(
        inner: Weak<RouterInner>,
        session_id: SessionId,
        run_id: RunId,
        user_id: String,
        cancel: CancellationToken,
        background_allowed_tools: Option<Arc<HashSet<String>>>,
    ) -> Self {
        Self {
            inner,
            session_id,
            run_id,
            user_id,
            cancel,
            background_allowed_tools,
            last_actor: Mutex::new(None),
        }
    }
}
