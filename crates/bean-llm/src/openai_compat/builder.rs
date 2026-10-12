use serde_json::{Value, json};

use crate::ChatRequest;
use crate::LlmError;
use crate::schema::sanitize_tool_schema;
use bean_types::message::Role;
use bean_types::{ALLOWED_IMAGE_MEDIA_TYPES, ImageBlock};

/// Từ chối danh sách messages rỗng **trước khi** dựng body.
///
/// Anthropic và OpenAI đều trả 400 với `messages: []`. Trả [`LlmError::Config`] rõ ràng
/// ở đây tốt hơn để lỗi HTTP khó hiểu lọt tới người dùng (D8.10 giữ lưới an toàn này ở
/// `run_turn` vì cùng lý do). Đây là tầng validate; [`build_request_body`] cố ý **thuần**
/// (không validate) để đường build/stream dùng chung.
///
/// # Errors
/// [`LlmError::Config`] khi `req.messages` rỗng.
pub fn ensure_non_empty(req: &ChatRequest<'_>) -> Result<(), LlmError> {
    if req.messages.is_empty() {
        return Err(LlmError::Config("danh sách messages rỗng".to_string()));
    }
    Ok(())
}

pub fn build_request_body(req: ChatRequest<'_>, model: &str, stream: bool) -> Value {
    let mut messages = Vec::with_capacity(req.messages.len() + 1);

    // 1. System message (nếu có).
    if !req.system.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": req.system
        }));
    }

    // 2. Chat history.
    for m in req.messages {
        match m.role {
            Role::User => {
                if let Some(text) = &m.text {
                    messages.push(json!({
                        "role": "user",
                        "content": text
                    }));
                } else {
                    // OpenAI compat provider: chưa có tool input builder cho file/image ở v1.
                    // Nếu gặp message User rỗng text (có thể do lỗi upstream), bỏ qua.
                }
            }
            Role::Assistant => {
                let mut wire = json!({"role": "assistant"});
                if let Some(text) = &m.text {
                    wire["content"] = json!(text);
                }
                if !m.tool_calls.is_empty() {
                    let calls: Vec<_> = m
                        .tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": c.args.to_string()
                                }
                            })
                        })
                        .collect();
                    wire["tool_calls"] = json!(calls);
                }
                messages.push(wire);
            }
            Role::Tool => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.text.clone().unwrap_or_default()
                }));
                // (M26) Chat Completions bắt buộc `role=tool` có `content` là chuỗi, nên ảnh
                // không nằm trong đó được — thêm message `user` kế tiếp, `content` mảng part.
                if let Some(image) = &m.image {
                    messages.push(image_user_message(image));
                }
            }
        }
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": stream
    });

    if req.max_tokens > 0 {
        // OpenAI model `o1-preview` chỉ chấp nhận `max_completion_tokens`.
        // Tuy nhiên các provider compat (Ollama, vLLM) và model cũ hơn lại chỉ biết `max_tokens`.
        // Để giữ tính compat rộng nhất, ta dùng trường `max_tokens` truyền thống.
        body["max_tokens"] = json!(req.max_tokens);
    }

    if !req.tools.is_empty() {
        let tools: Vec<_> = req
            .tools
            .iter()
            .map(|t| {
                let mut parameters = t.parameters.clone();
                sanitize_tool_schema(&mut parameters);
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": parameters
                    }
                })
            })
            .collect();
        body["tools"] = json!(tools);
    }

    body
}

/// `ImageBlock` → message `user` chứa ảnh (Chat Completions, M26): `content` là mảng part
/// `{"type":"image_url","image_url":{"url":"data:…"}}`.
///
/// `media_type` được kiểm lại: lịch sử cũ (đọc từ DB) có thể ghi media type lạ, và một media
/// type lạ sẽ khiến API trả 400 — hỏng cả lượt chat chứ không chỉ tool ảnh.
fn image_user_message(image: &ImageBlock) -> Value {
    let media_type = if ALLOWED_IMAGE_MEDIA_TYPES.contains(&image.media_type.as_str()) {
        image.media_type.as_str()
    } else {
        "image/png"
    };
    json!({
        "role": "user",
        "content": [{
            "type": "image_url",
            "image_url": { "url": format!("data:{media_type};base64,{}", image.data) }
        }]
    })
}
