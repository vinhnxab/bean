use serde_json::Value;

use bean_types::ToolCall;
use bean_types::llm::{LlmDelta, LlmResponse, LlmToolCallDelta, StopReason, Usage};

use super::schema::{WireResponse, WireUsage};
use crate::LlmError;
use crate::stream::{SseEvent, SseEventParser};

pub struct OpenAiStreamParser {
    pub usage: Option<Usage>,
    pub stop: Option<StopReason>,
    pub done: bool,
}

impl SseEventParser for OpenAiStreamParser {
    fn parse_event(&mut self, event: SseEvent) -> Result<Vec<LlmDelta>, LlmError> {
        if event.data == "[DONE]" {
            self.done = true;
            return Ok(Vec::new());
        }

        let wire: Value = serde_json::from_str(&event.data)
            .map_err(|e| LlmError::Decode(format!("SSE JSON sai: {e}")))?;

        let mut deltas = Vec::new();

        // 1. Trích xuất usage (thường ở chunk cuối).
        if let Some(usage) = wire.get("usage") {
            let u: WireUsage = serde_json::from_value(usage.clone())
                .map_err(|e| LlmError::Decode(format!("Usage JSON sai: {e}")))?;
            let usage = Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            };
            self.usage = Some(usage);
            deltas.push(LlmDelta::Usage { usage });
        }

        // 2. Trích xuất delta từ choice đầu tiên.
        let choices = wire.get("choices").and_then(|v| v.as_array());
        let choice = choices.and_then(|a| a.first());

        if let Some(choice) = choice {
            if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                let stop = map_finish_reason(reason);
                self.stop = Some(stop);
                deltas.push(LlmDelta::Stop { reason: stop });
            }

            let delta = choice.get("delta");
            if let Some(delta) = delta {
                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    deltas.push(LlmDelta::Text {
                        text: text.to_string(),
                    });
                }

                if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                    for c in calls {
                        let index = c.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let id = c.get("id").and_then(|v| v.as_str()).map(String::from);
                        let function = c.get("function");
                        let name = function
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let arguments_delta = function
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .map(String::from);

                        deltas.push(LlmDelta::ToolCall(LlmToolCallDelta {
                            index,
                            id,
                            name,
                            arguments_delta,
                        }));
                    }
                }
            }
        }

        Ok(deltas)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(&mut self) -> Result<(), LlmError> {
        Ok(())
    }
}

pub fn parse_response(text: &str) -> Result<LlmResponse, LlmError> {
    let wire: WireResponse = serde_json::from_str(text)
        .map_err(|e| LlmError::Decode(format!("OpenAI JSON sai: {e}")))?;
    parse_wire_response(wire)
}

/// Tầng dưới: nhận [`WireResponse`] đã deserialize.
///
/// Tách khỏi [`parse_response`] để đường streaming (SSE, M17) và mọi đường đã có sẵn
/// struct `WireResponse` có thể tái dùng mà không phải deserialize lại từ chuỗi.
pub fn parse_wire_response(wire: WireResponse) -> Result<LlmResponse, LlmError> {
    let choice = wire
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::Decode("OpenAI trả về danh sách choice rỗng".into()))?;

    let mut tool_calls = Vec::new();
    if let Some(calls) = choice.message.tool_calls {
        for c in calls {
            tool_calls.push(ToolCall {
                id: c.id,
                name: c.function.name,
                args: parse_arguments(c.function.arguments.as_ref())?,
            });
        }
    }

    let stop = choice
        .finish_reason
        .as_deref()
        .map(map_finish_reason)
        .unwrap_or(StopReason::Other);

    let usage = wire
        .usage
        .map(|u| Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        })
        .unwrap_or_default();

    Ok(LlmResponse {
        text: choice.message.content,
        tool_calls,
        stop,
        usage,
    })
}

/// `arguments` theo spec OpenAI là **chuỗi JSON**; server lệch chuẩn (một số server local)
/// có thể trả `null` hoặc object. Chuẩn hoá: rỗng/null ⇒ `{}`; chuỗi ⇒ parse JSON;
/// object/kiểu khác ⇒ dùng nguyên (không lỗi) — hành vi này giữ tương thích rộng.
fn parse_arguments(raw: Option<&Value>) -> Result<Value, LlmError> {
    match raw {
        None | Some(Value::Null) => Ok(Value::Object(serde_json::Map::new())),
        Some(Value::String(text)) if text.trim().is_empty() => {
            Ok(Value::Object(serde_json::Map::new()))
        }
        Some(Value::String(text)) => serde_json::from_str::<Value>(text).map_err(|e| {
            LlmError::Decode(format!(
                "không parse được arguments của tool (OpenAI compat trả về chuỗi JSON sai): {e}"
            ))
        }),
        Some(other) => Ok(other.clone()),
    }
}

pub fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        _ => StopReason::Other,
    }
}
