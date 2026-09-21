//! FakeProvider — trả lần lượt danh sách [`LlmResponse`] dựng sẵn.
//!
//! Dùng cho **mọi** test của vòng lặp agent (không mạng, không tốn tiền) và cho
//! `BeanAgent serve --fake-llm kichban.json` (agents.md mục 5, 20).
//!
//! Định dạng file kịch bản (JSON):
//!
//! ```json
//! {
//!   "responses": [
//!     { "text": "Xin chào", "stop": "end_turn" },
//!     { "tool_calls": [ { "id": "c1", "name": "read_file", "args": { "path": "a.txt" } } ],
//!       "stop": "tool_use" }
//!   ]
//! }
//! ```
//!
//! Cũng chấp nhận dạng mảng trần `[ { … }, { … } ]`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use beanagent_types::llm::LlmResponse;
use beanagent_types::message::Role;
use serde::{Deserialize, Serialize};

use crate::{ChatRequest, LlmError, LlmProvider};

/// Nội dung file kịch bản.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FakeScript {
    /// Danh sách response trả lần lượt.
    #[serde(default)]
    pub responses: Vec<LlmResponse>,
}

/// Provider giả: trả lần lượt các response trong kịch bản, hoặc echo nếu kịch bản rỗng.
#[derive(Debug)]
pub struct FakeProvider {
    responses: Vec<LlmResponse>,
    cursor: AtomicUsize,
}

impl FakeProvider {
    /// Provider trả lần lượt `responses` (hết thì trả lỗi, không panic).
    #[must_use]
    pub fn new(responses: Vec<LlmResponse>) -> Self {
        Self {
            responses,
            cursor: AtomicUsize::new(0),
        }
    }

    /// Provider echo: trả lại nguyên văn tin nhắn cuối của người dùng.
    ///
    /// Dùng cho `BeanAgent chat` khi chưa có API key (agents.md mục 21, M1).
    #[must_use]
    pub fn echo() -> Self {
        Self::new(Vec::new())
    }

    /// Nạp kịch bản từ chuỗi JSON.
    ///
    /// # Errors
    /// Trả [`LlmError::FakeScript`] khi JSON không hợp lệ hoặc thiếu trường bắt buộc.
    pub fn from_json_str(json: &str) -> Result<Self, LlmError> {
        if let Ok(responses) = serde_json::from_str::<Vec<LlmResponse>>(json) {
            return Ok(Self::new(responses));
        }
        let script: FakeScript =
            serde_json::from_str(json).map_err(|e| LlmError::FakeScript(e.to_string()))?;
        Ok(Self::new(script.responses))
    }

    /// Nạp kịch bản từ file.
    ///
    /// # Errors
    /// Trả [`LlmError::FakeScript`] khi không đọc được file hoặc JSON không hợp lệ.
    pub fn from_json_path(path: &Path) -> Result<Self, LlmError> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            LlmError::FakeScript(format!("không đọc được `{}`: {e}", path.display()))
        })?;
        Self::from_json_str(&raw)
    }

    /// Số response còn lại trong kịch bản.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.responses
            .len()
            .saturating_sub(self.cursor.load(Ordering::Relaxed))
    }

    /// Provider này có phải chế độ echo không?
    #[must_use]
    pub fn is_echo(&self) -> bool {
        self.responses.is_empty()
    }
}

#[async_trait]
impl LlmProvider for FakeProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        if self.responses.is_empty() {
            let last_user = req
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .and_then(|m| m.text.clone())
                .unwrap_or_default();
            return Ok(LlmResponse::text_only(format!("echo: {last_user}")));
        }

        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(index)
            .cloned()
            .ok_or(LlmError::FakeScriptExhausted {
                requested: index + 1,
                available: self.responses.len(),
            })
    }

    fn name(&self) -> &'static str {
        "fake"
    }
}
