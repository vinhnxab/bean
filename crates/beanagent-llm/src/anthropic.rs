//! Provider **Anthropic Messages API** (agents.md mục 5).
//!
//! Đặc tả wire (đối chiếu docs chính thức khi code — agents.md mục 0.3):
//! * `POST {base}/v1/messages`, header `x-api-key` + `anthropic-version: 2023-06-01`.
//! * `system` là **trường riêng** của request, không nằm trong `messages`.
//! * Assistant trả `content: [{type:"text"} | {type:"tool_use", id, name, input}]`.
//! * Kết quả tool là message `user` với block `tool_result` (`tool_use_id`, `content`,
//!   `is_error`); **nhiều** kết quả song song được ghép vào **một** message `user`.
//! * `stop_reason`: `end_turn` | `tool_use` | `max_tokens` | còn lại → `Other`.
//!
//! Mỗi lượt [`LlmProvider::chat`] phát đúng **một** request; retry/backoff nằm ở
//! [`crate::retry`] (tách lớp, xem `docs/decisions.md` D6.1).

use std::fmt;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use beanagent_types::llm::{LlmResponse, StopReason, Usage};
use beanagent_types::message::Role;
use beanagent_types::{ToolCall, ToolSpec};

use crate::schema::sanitize_tool_schema;
use crate::{ChatRequest, LlmError, LlmProvider, http, retry};

/// Giá trị header `anthropic-version` hiện hành (docs.anthropic.com).
const API_VERSION: &str = "2023-06-01";

/// Endpoint chính thức; ghi đè bằng `llm.base_url` (cho test/gateway).
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Provider Anthropic.
///
/// API key được bọc [`SecretString`]; `Debug` chỉ in `<redacted>` (agents.md mục 15.6).
pub struct AnthropicProvider {
    endpoint: String,
    api_key: SecretString,
    model: String,
    client: reqwest::Client,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl AnthropicProvider {
    /// Tạo provider.
    ///
    /// # Errors
    /// [`LlmError::Config`] khi không dựng được HTTP client.
    pub fn new(
        model: impl Into<String>,
        api_key: SecretString,
        base_url: Option<&str>,
    ) -> Result<Self, LlmError> {
        Ok(Self {
            endpoint: endpoint_url(base_url),
            api_key,
            model: model.into(),
            client: http::build_client(http::DEFAULT_TIMEOUT)?,
        })
    }
}

/// `{base}` + `/v1/messages`. Chấp nhận cả ba dạng `base_url` mà người dùng hay viết:
/// `https://api.anthropic.com` (root), `…/v1` (thói quen từ OpenAI), hoặc `…/v1/messages`
/// (trỏ thẳng) — tránh lỗi cổ điển `…/v1/v1/messages`.
fn endpoint_url(base_url: Option<&str>) -> String {
    let base = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/');
    if base.ends_with("/messages") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        let body = build_request_body(&req, &self.model)?;
        let endpoint = self.endpoint.clone();
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        retry::retry_with_backoff(retry::BASE_DELAY, retry::MAX_RETRIES, move || {
            let body = body.clone();
            let api_key = api_key.clone();
            let endpoint = endpoint.clone();
            let client = client.clone();
            async move {
                let response = client
                    .post(&endpoint)
                    .header("x-api-key", api_key.expose_secret())
                    .header("anthropic-version", API_VERSION)
                    .json(&body)
                    .send()
                    .await
                    .map_err(http::to_transport_error)?;

                let status = response.status();
                let retry_after = http::parse_retry_after(response.headers());
                let text = response
                    .text()
                    .await
                    .map_err(|err| LlmError::Transport(err.to_string()))?;

                if status.as_u16() == 429 {
                    return Err(LlmError::RateLimited { retry_after });
                }
                if !status.is_success() {
                    return Err(LlmError::HttpStatus {
                        status: status.as_u16(),
                        body: http::truncate_body(&text),
                    });
                }
                parse_response(&text)
            }
        })
        .await
    }

    fn name(&self) -> &'static str {
        "anthropic"
    }
}

/// Dựng body request. `Role::Tool` liên tiếp được ghép thành **một** message `user`
/// chứa nhiều block `tool_result` (định dạng đúng cho các tool call song song).
///
/// # Errors
/// [`LlmError::Config`] khi message `tool` thiếu `tool_call_id` hoặc danh sách rỗng.
pub fn build_request_body(req: &ChatRequest<'_>, model: &str) -> Result<Value, LlmError> {
    let mut messages: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();

    for message in req.messages {
        match message.role {
            Role::User => {
                flush_tool_results(&mut messages, &mut pending_tool_results);
                messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": message.text.as_deref().unwrap_or_default() }]
                }));
            }
            Role::Assistant => {
                flush_tool_results(&mut messages, &mut pending_tool_results);
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = message.text.as_deref().filter(|t| !t.is_empty()) {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
                for call in &message.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.args,
                    }));
                }
                if blocks.is_empty() {
                    // Assistant "rỗng" — Anthropic từ chối content rỗng.
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                messages.push(json!({ "role": "assistant", "content": blocks }));
            }
            Role::Tool => {
                let Some(id) = message.tool_call_id.as_deref() else {
                    return Err(LlmError::Config(
                        "message role=tool thiếu tool_call_id — lịch sử bị hỏng cặp tool_use/tool_result"
                            .to_string(),
                    ));
                };
                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": message.text.as_deref().unwrap_or_default(),
                    "is_error": message.is_error,
                }));
            }
        }
    }
    flush_tool_results(&mut messages, &mut pending_tool_results);

    if messages.is_empty() {
        return Err(LlmError::Config("danh sách messages rỗng".to_string()));
    }

    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "messages": messages,
    });
    if !req.system.is_empty() {
        body["system"] = json!(req.system);
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req.tools.iter().map(tool_to_wire).collect();
        body["tools"] = Value::Array(tools);
    }
    Ok(body)
}

fn flush_tool_results(messages: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if !pending.is_empty() {
        messages.push(json!({ "role": "user", "content": pending }));
        pending.clear();
    }
}

/// `ToolSpec` → `{"name", "description", "input_schema"}` với schema đã chuẩn hoá.
fn tool_to_wire(tool: &ToolSpec) -> Value {
    let mut parameters = tool.parameters.clone();
    sanitize_tool_schema(&mut parameters);
    json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": parameters,
    })
}

#[derive(Deserialize)]
struct WireResponse {
    #[serde(default)]
    content: Vec<WireContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: WireUsage,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireContentBlock {
    /// Block văn bản.
    Text { text: String },
    /// Block yêu cầu gọi tool.
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    /// Block khác (vd `thinking`) — bỏ qua, không lỗi.
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

/// # Errors
/// [`LlmError::Decode`] khi JSON sai định dạng.
pub fn parse_response(text: &str) -> Result<LlmResponse, LlmError> {
    let wire: WireResponse = serde_json::from_str(text)
        .map_err(|err| LlmError::Decode(format!("phản hồi Anthropic sai định dạng: {err}")))?;

    let mut text_out = String::new();
    let mut tool_calls = Vec::new();
    for block in &wire.content {
        match block {
            WireContentBlock::Text { text } => {
                if !text_out.is_empty() {
                    text_out.push('\n');
                }
                text_out.push_str(text);
            }
            WireContentBlock::ToolUse { id, name, input } => {
                let args = if input.is_null() {
                    json!({})
                } else {
                    input.clone()
                };
                tool_calls.push(ToolCall::new(id.clone(), name.clone(), args));
            }
            WireContentBlock::Other => {
                tracing::debug!("bỏ qua content block không nhận diện được từ Anthropic");
            }
        }
    }

    Ok(LlmResponse {
        text: if text_out.is_empty() {
            None
        } else {
            Some(text_out)
        },
        tool_calls,
        stop: map_stop_reason(wire.stop_reason.as_deref()),
        usage: Usage {
            input_tokens: wire.usage.input_tokens,
            output_tokens: wire.usage.output_tokens,
        },
    })
}

fn map_stop_reason(raw: Option<&str>) -> StopReason {
    match raw {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        // stop_sequence / refusal / pause_turn / giá trị tương lai → Other.
        _ => StopReason::Other,
    }
}
