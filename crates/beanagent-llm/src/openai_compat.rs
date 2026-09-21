//! Provider cho **mọi endpoint tương thích OpenAI Chat Completions** (agents.md mục 5).
//!
//! Đặc tả wire:
//! * `POST {base}/chat/completions`; `base_url` mặc định `https://api.openai.com/v1`
//!   (nên mọi base tuỳ biến đều chỉ cần tới mức `/v1`).
//! * Header `Authorization: Bearer <key>`; **bỏ qua** header khi key rỗng (Ollama/vLLM).
//! * `tool_calls[].function.arguments` là **chuỗi JSON** (khác Anthropic: `input` là object) —
//!   parse thất bại là [`LlmError::Decode`], không panic.
//! * Gửi `max_tokens` (trường kinh điển được mọi server compat chấp nhận); **không** gửi
//!   `max_completion_tokens` đồng thời — OpenAI từ chối request khi có cả hai.
//! * Message `tool` → `{"role":"tool","tool_call_id":…,"content":…}`.
//!
//! Mỗi lượt [`LlmProvider::chat`] phát đúng **một** request; retry nằm ở [`crate::retry`].

use std::fmt;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use beanagent_types::ToolCall;
use beanagent_types::llm::{LlmResponse, StopReason, Usage};
use beanagent_types::message::Role;

use crate::schema::sanitize_tool_schema;
use crate::{ChatRequest, LlmError, LlmProvider, http, retry};

/// Endpoint chính thức (đã gồm `/v1`); ghi đè bằng `llm.base_url`.
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Provider OpenAI-compat.
///
/// API key được bọc [`SecretString`]; `Debug` chỉ in `<redacted>` (agents.md mục 15.6).
pub struct OpenAiCompatProvider {
    endpoint: String,
    api_key: Option<SecretString>,
    model: String,
    client: reqwest::Client,
}

impl fmt::Debug for OpenAiCompatProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatProvider")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

impl OpenAiCompatProvider {
    /// Tạo provider.
    ///
    /// * `api_key` rỗng được coi như không có key.
    /// * Không key mà cũng không `base_url` → mặc định là OpenAI chính thức → lỗi.
    ///
    /// # Errors
    /// [`LlmError::Config`] khi thiếu key cho endpoint chính thức hoặc client dựng thất bại.
    pub fn new(
        model: impl Into<String>,
        api_key: Option<SecretString>,
        base_url: Option<&str>,
    ) -> Result<Self, LlmError> {
        let key = api_key.filter(|k| !k.expose_secret().is_empty());
        if key.is_none() && base_url.is_none() {
            return Err(LlmError::Config(
                "provider `openai_compat` trỏ tới OpenAI chính thức nên cần API key; \
                 nếu dùng server tự host (Ollama/vLLM) hãy đặt `llm.base_url`"
                    .to_string(),
            ));
        }
        Ok(Self {
            endpoint: endpoint_url(base_url),
            api_key: key,
            model: model.into(),
            client: http::build_client(http::DEFAULT_TIMEOUT)?,
        })
    }
}

/// `{base}` + `/chat/completions`; nếu `base_url` đã trỏ thẳng tới `…/chat/completions`
/// thì giữ nguyên.
fn endpoint_url(base_url: Option<&str>) -> String {
    let base = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
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
                let mut request = client.post(&endpoint);
                if let Some(key) = &api_key {
                    request = request.bearer_auth(key.expose_secret());
                }
                let response = request
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
                        body: error_body(&text),
                    });
                }
                parse_response(&text)
            }
        })
        .await
    }

    fn name(&self) -> &'static str {
        "openai_compat"
    }
}

/// Dựng body request Chat Completions.
///
/// # Errors
/// [`LlmError::Config`] khi message `tool` thiếu `tool_call_id` hoặc danh sách rỗng.
pub fn build_request_body(req: &ChatRequest<'_>, model: &str) -> Result<Value, LlmError> {
    if req.messages.is_empty() {
        // Nhất quán với provider Anthropic: chỉ có system mà không có lượt hội thoại
        // nào là lỗi cấu hình lịch sử, không phải request hợp lệ.
        return Err(LlmError::Config("danh sách messages rỗng".to_string()));
    }

    let mut messages: Vec<Value> = Vec::new();
    if !req.system.is_empty() {
        messages.push(json!({ "role": "system", "content": req.system }));
    }

    for message in req.messages {
        match message.role {
            Role::User => {
                messages.push(json!({
                    "role": "user",
                    "content": message.text.as_deref().unwrap_or_default(),
                }));
            }
            Role::Assistant => {
                let mut entry = json!({ "role": "assistant" });
                if let Some(text) = message.text.as_deref().filter(|t| !t.is_empty()) {
                    entry["content"] = json!(text);
                } else if message.tool_calls.is_empty() {
                    entry["content"] = json!("");
                } else {
                    // Chỉ gọi tool: content null là hợp lệ theo spec OpenAI.
                    entry["content"] = Value::Null;
                }
                if !message.tool_calls.is_empty() {
                    let calls: Vec<Value> = message
                        .tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    // ⚠️ arguments phải là CHUỖI JSON, không phải object.
                                    "arguments": call.args.to_string(),
                                },
                            })
                        })
                        .collect();
                    entry["tool_calls"] = Value::Array(calls);
                }
                messages.push(entry);
            }
            Role::Tool => {
                let Some(id) = message.tool_call_id.as_deref() else {
                    return Err(LlmError::Config(
                        "message role=tool thiếu tool_call_id — lịch sử bị hỏng cặp tool_calls/tool"
                            .to_string(),
                    ));
                };
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": message.text.as_deref().unwrap_or_default(),
                }));
            }
        }
    }

    if messages.is_empty() {
        return Err(LlmError::Config("danh sách messages rỗng".to_string()));
    }

    let mut body = json!({
        "model": model,
        "max_tokens": req.max_tokens,
        "messages": messages,
    });
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|tool| {
                let mut parameters = tool.parameters.clone();
                sanitize_tool_schema(&mut parameters);
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": parameters,
                    },
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct WireResponse {
    #[serde(default)]
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: WireUsage,
}

#[derive(Deserialize, Default)]
struct WireChoice {
    #[serde(default)]
    message: WireMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct WireMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCall>,
}

#[derive(Deserialize)]
struct WireToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<WireFunction>,
}

#[derive(Deserialize)]
struct WireFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Deserialize, Default)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

/// # Errors
/// [`LlmError::Decode`] khi JSON sai định dạng, thiếu `choices`, hoặc `arguments`
/// không parse được thành JSON.
pub fn parse_response(text: &str) -> Result<LlmResponse, LlmError> {
    let wire: WireResponse = serde_json::from_str(text)
        .map_err(|err| LlmError::Decode(format!("phản hồi OpenAI-compat sai định dạng: {err}")))?;
    let choice = wire
        .choices
        .first()
        .ok_or_else(|| LlmError::Decode("phản hồi OpenAI-compat không có choices".to_string()))?;

    let mut tool_calls = Vec::new();
    for (index, call) in choice.message.tool_calls.iter().enumerate() {
        let Some(function) = call.function.as_ref() else {
            return Err(LlmError::Decode(format!(
                "tool_calls[{index}] thiếu trường function"
            )));
        };
        let Some(name) = function.name.as_deref().filter(|n| !n.is_empty()) else {
            return Err(LlmError::Decode(format!(
                "tool_calls[{index}] thiếu function.name"
            )));
        };
        let args = parse_arguments(index, function.arguments.as_ref())?;
        let id = call
            .id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| format!("call_{index}"));
        tool_calls.push(ToolCall::new(id, name, args));
    }

    let text = choice
        .message
        .content
        .as_deref()
        .filter(|t| !t.is_empty())
        .map(str::to_string);

    Ok(LlmResponse {
        text,
        tool_calls,
        stop: map_finish_reason(choice.finish_reason.as_deref()),
        usage: Usage {
            input_tokens: wire.usage.prompt_tokens,
            output_tokens: wire.usage.completion_tokens,
        },
    })
}

/// `arguments` là **chuỗi JSON** theo spec; server lệch chuẩn trả object cũng chấp nhận.
fn parse_arguments(index: usize, raw: Option<&Value>) -> Result<Value, LlmError> {
    match raw {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(json!({})),
        Some(Value::String(text)) => serde_json::from_str::<Value>(text).map_err(|err| {
            LlmError::Decode(format!(
                "tool_calls[{index}].function.arguments không phải JSON hợp lệ: {err}; nội dung: {}",
                http::truncate_body(text)
            ))
        }),
        Some(other) => Ok(other.clone()),
    }
}

fn map_finish_reason(raw: Option<&str>) -> StopReason {
    match raw {
        Some("stop") => StopReason::EndTurn,
        Some("tool_calls" | "function_call") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        // content_filter / null / giá trị tương lai → Other.
        _ => StopReason::Other,
    }
}

/// Trích `error.message` (shape lỗi chuẩn của OpenAI) nếu có, giữ body thô nếu không.
fn error_body(text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text)
        && let Some(message) = value
            .get("error")
            .and_then(|err| err.get("message"))
            .and_then(Value::as_str)
    {
        return http::truncate_body(message);
    }
    http::truncate_body(text)
}
