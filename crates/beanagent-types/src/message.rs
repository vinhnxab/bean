//! Định dạng message trung lập với provider (agents.md mục 5).
//!
//! Bất biến quan trọng: một `Assistant` có `tool_calls` luôn phải đi kèm các `Tool` result
//! **đủ và đúng thứ tự** (`tool_call_id` khớp). Mọi thao tác cắt lịch sử trong
//! `BeanAgent-memory` phải giữ nguyên cặp này, nếu không API sẽ trả lỗi 400
//! (agents.md mục 8.3, 22.1).

use serde::{Deserialize, Serialize};

use crate::llm::LlmResponse;

/// Vai trò của message.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Người dùng.
    User,
    /// Model (có thể kèm `tool_calls`).
    Assistant,
    /// Kết quả tool trả lại cho model.
    Tool,
}

impl Role {
    /// Chuỗi dùng trong cột `messages.role` của SQLite.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// Một yêu cầu gọi tool do model phát ra.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolCall {
    /// Id do provider sinh; phải được giữ nguyên khi gửi lại tool result.
    pub id: String,
    /// Tên tool, khớp `ToolSpec::name`.
    pub name: String,
    /// Tham số thô (`serde_json::Value`) — là đầu vào **không tin cậy**, phải validate trước khi dùng.
    pub args: serde_json::Value,
}

impl ToolCall {
    /// Tạo một tool call.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            args,
        }
    }
}

/// Một khối **ảnh** đi kèm message (M26 — tool `browser_screenshot`).
///
/// Base64 **chỉ** tồn tại ở hai chỗ: trong `content_json` của SQLite (để phát lại lịch
/// sử cho model ở lượt sau) và trong response REST tới UI. Nó **không** bao giờ đi vào
/// `audit.jsonl` — audit chỉ ghi [`Self::sha256`] (mục 15.8, `docs/known-issues.md`).
///
/// `media_type` là MIME do **Chromium trả về** (`Page.captureScreenshot.format`), không
/// phải chuỗi tự do của model: hàm dựng bị chặn ở tập MIME được phép.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ImageBlock {
    /// MIME của ảnh, ví dụ `image/png`.
    pub media_type: String,
    /// Ảnh đã mã hoá base64 (không có padding `=` trong JSON — xem `base64` crate).
    pub data: String,
    /// SHA-256 (hex) của **byte ảnh gốc**, dùng làm tham chiếu trong audit.
    pub sha256: String,
}

/// MIME ảnh được phép đưa tới provider.
///
/// **Fail-closed**: chỉ những gì `Page.captureScreenshot` thật sự trả về. Một
/// `media_type` lạ (SVG, HTML) sẽ bị provider từ chối, hoặc tệ hơn là bị diễn giải
/// sai — nên chặn ngay ở tầng dựng khối thay vì để lọt xuống tầng HTTP.
pub const ALLOWED_IMAGE_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp"];

impl ImageBlock {
    /// Dựng khối ảnh, **từ chối** `media_type` nằm ngoài [`ALLOWED_IMAGE_MEDIA_TYPES`].
    ///
    /// `data` phải là base64 đã mã hoá và `sha256` là hex của **byte gốc** (không phải
    /// của chuỗi base64) — như vậy hai lần chụp cùng một trang cho cùng một tham chiếu
    /// trong audit, và việc kiểm chứng lại không cần giải mã.
    ///
    /// # Errors
    /// `Err` với lý do nếu `media_type` không được phép.
    pub fn new(
        media_type: impl Into<String>,
        data: impl Into<String>,
        sha256: impl Into<String>,
    ) -> Result<Self, String> {
        let media_type = media_type.into();
        if !ALLOWED_IMAGE_MEDIA_TYPES.contains(&media_type.as_str()) {
            return Err(format!(
                "media_type `{media_type}` không được phép; chỉ nhận: {}",
                ALLOWED_IMAGE_MEDIA_TYPES.join(", ")
            ));
        }
        Ok(Self {
            media_type,
            data: data.into(),
            sha256: sha256.into(),
        })
    }
}

/// Một message trong hội thoại.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub role: Role,
    pub text: Option<String>,
    /// Chỉ có ý nghĩa với `Assistant`.
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Chỉ có ý nghĩa với `Tool`.
    pub tool_call_id: Option<String>,
    /// Chỉ có ý nghĩa với `Tool`: đây là lỗi (để model đọc và tự sửa, agents.md mục 6).
    #[serde(default)]
    pub is_error: bool,
    /// Ảnh đi kèm, chỉ có ở kết quả tool trả ảnh (M26).
    ///
    /// `skip_serializing_if` giữ cho **mọi message cũ không có trường này** serialize
    /// y hệt trước đây — lịch sử đã ghi vẫn đọc được và `git diff` kiểu TS không
    /// đổi. `#[serde(default)]` là chiều ngược: message không có ảnh vẫn deserialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageBlock>,
}

impl Message {
    /// Message của người dùng.
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            text: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            is_error: false,
            image: None,
        }
    }

    /// Message của model (có thể vừa có text vừa có `tool_calls`).
    #[must_use]
    pub fn assistant(text: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            text,
            tool_calls,
            tool_call_id: None,
            is_error: false,
            image: None,
        }
    }

    /// Kết quả tool thành công.
    #[must_use]
    pub fn tool(tool_call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            text: Some(text.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            is_error: false,
            image: None,
        }
    }

    /// Kết quả tool thất bại (kể cả bị huỷ, timeout, sai tham số).
    #[must_use]
    pub fn tool_error(tool_call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            ..Self::tool(tool_call_id, text)
        }
    }

    /// Kết quả tool **có kèm ảnh** (M26 — `browser_screenshot`).
    ///
    /// `caption` là dòng mô tả ngắn đi cùng ảnh: provider nào cũng cần ít nhất một
    /// block text để model biết đang nhìn cái gì, và nó là thứ được tính vào ngân sách
    /// token theo công thức `chars/4` sẵn có (ảnh thì dùng định phí riêng).
    #[must_use]
    pub fn tool_with_image(
        tool_call_id: impl Into<String>,
        caption: impl Into<String>,
        image: ImageBlock,
    ) -> Self {
        Self {
            image: Some(image),
            ..Self::tool(tool_call_id, caption)
        }
    }

    /// Chuyển một `LlmResponse` thành message của assistant để ghi vào lịch sử.
    #[must_use]
    pub fn from_response(resp: &LlmResponse) -> Self {
        Self::assistant(resp.text.clone(), resp.tool_calls.clone())
    }

    /// Văn bản dùng cho cột `text_for_search` (FTS5, agents.md mục 8.1).
    ///
    /// Gộp phần text nhìn thấy được và tên/đối số tool để tìm kiếm được cả hành động.
    #[must_use]
    pub fn text_for_search(&self) -> String {
        let mut out = self.text.clone().unwrap_or_default();
        for call in &self.tool_calls {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[tool:{}] {}", call.name, call.args));
        }
        out
    }
}
