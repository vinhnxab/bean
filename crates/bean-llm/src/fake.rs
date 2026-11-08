//! FakeProvider — trả lần lượt danh sách [`LlmResponse`] dựng sẵn.
//!
//! Dùng cho **mọi** test của vòng lặp agent (không mạng, không tốn tiền) và cho
//! `bean serve --fake-llm kichban.json` (agents.md mục 5, 20).
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
use bean_types::llm::{LlmResponse, Usage};
use bean_types::message::Role;
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

/// Ước lượng token theo `chars/4` (agents.md mục 8.2).
///
/// Provider thật luôn báo usage; kịch bản fake không khai báo usage thì tự dựng
/// số liệu AllY deterministic để metrics eval (số token, ngân sách) đo được
/// thay vì luôn bằng 0. Kịch bản khai báo usage tường minh thì được giữ nguyên.
fn estimate_usage(req: &ChatRequest<'_>, response: &LlmResponse) -> Usage {
    let mut input = req.system.len();
    for message in req.messages {
        input += message.text.as_deref().map_or(0, str::len);
        for call in &message.tool_calls {
            input += call.name.len();
            input += serde_json::to_string(&call.args).map_or(0, |json| json.len());
        }
    }
    let mut output = response.text.as_deref().map_or(0, str::len);
    for call in &response.tool_calls {
        output += call.name.len();
        output += serde_json::to_string(&call.args).map_or(0, |json| json.len());
    }
    Usage {
        input_tokens: tokens_from_chars(input),
        output_tokens: tokens_from_chars(output),
    }
}

fn tokens_from_chars(chars: usize) -> u32 {
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
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
    /// Dùng cho `bean chat` khi chưa có API key (agents.md mục 21, M1).
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
            let response = LlmResponse::text_only(format!("echo: {last_user}"));
            let usage = estimate_usage(&req, &response);
            return Ok(LlmResponse { usage, ..response });
        }

        let index = self.cursor.fetch_add(1, Ordering::SeqCst);
        let mut response =
            self.responses
                .get(index)
                .cloned()
                .ok_or(LlmError::FakeScriptExhausted {
                    requested: index + 1,
                    available: self.responses.len(),
                })?;
        if response.usage.input_tokens == 0 && response.usage.output_tokens == 0 {
            response.usage = estimate_usage(&req, &response);
        }
        Ok(response)
    }

    fn name(&self) -> &'static str {
        "fake"
    }
}
