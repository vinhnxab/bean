use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Debug)]
pub struct WireResponse {
    pub choices: Vec<WireChoice>,
    pub usage: Option<WireUsage>,
}

#[derive(Deserialize, Debug)]
pub struct WireChoice {
    pub message: WireMessage,
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct WireMessage {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Deserialize, Debug)]
pub struct WireToolCall {
    pub id: String,
    pub function: WireFunction,
}

#[derive(Deserialize, Debug)]
pub struct WireFunction {
    pub name: String,
    /// Theo spec OpenAI là **chuỗi JSON**; server lệch chuẩn (một số server local) có thể
    /// trả `null` hoặc object. `Option<Value>` chấp nhận cả ba dạng — `parser::parse_arguments`
    /// chuẩn hoá về object `{}`/đã parse. Thiếu field ⇒ `None`.
    #[serde(default)]
    pub arguments: Option<Value>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct WireUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}
