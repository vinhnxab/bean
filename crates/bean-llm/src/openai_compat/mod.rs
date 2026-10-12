use std::fmt;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use bean_types::llm::LlmResponse;

use crate::stream::decode_sse;
use crate::{ChatRequest, LlmError, LlmProvider, LlmStream, http, retry};

pub mod builder;
pub mod parser;
pub mod schema;

use self::builder::{build_request_body, ensure_non_empty};
use self::parser::{OpenAiStreamParser, parse_response};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Provider OpenAI-compat.
pub struct OpenAiCompatProvider {
    endpoint: String,
    api_key: Option<SecretString>,
    api_key_env: String,
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
    pub fn new(
        model: impl Into<String>,
        api_key: Option<SecretString>,
        base_url: Option<&str>,
        api_key_env: &str,
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
            api_key_env: api_key_env.to_string(),
            model: model.into(),
            client: http::build_client(http::DEFAULT_TIMEOUT)?,
        })
    }

    async fn open_stream(&self, body: Value) -> Result<reqwest::Response, LlmError> {
        let endpoint = self.endpoint.clone();
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let api_key_env = self.api_key_env.clone();
        retry::retry_with_backoff(retry::BASE_DELAY, retry::MAX_RETRIES, move || {
            let body = body.clone();
            let api_key = api_key.clone();
            let endpoint = endpoint.clone();
            let api_key_env = api_key_env.clone();
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
                if status.as_u16() == 429 {
                    return Err(LlmError::RateLimited { retry_after });
                }
                if !status.is_success() {
                    let text = response
                        .text()
                        .await
                        .map_err(|error| LlmError::Transport(error.to_string()))?;
                    return Err(status_error(
                        &endpoint,
                        api_key.is_none(),
                        &api_key_env,
                        status.as_u16(),
                        error_body(&text),
                    ));
                }
                Ok(response)
            }
        })
        .await
    }
}

fn status_error(
    endpoint: &str,
    key_absent: bool,
    api_key_env: &str,
    status: u16,
    body: String,
) -> LlmError {
    if key_absent && (status == 401 || status == 403) {
        return LlmError::Config(format!(
            "{endpoint} từ chối (HTTP {status}) và Bean KHÔNG gửi header `Authorization` vì \
             biến môi trường `{env}` đang rỗng hoặc chưa được đặt. Hãy `export {env}=...` \
             rồi khởi động lại Bean (tên biến lấy từ `llm.api_key_env` trong bean.toml). \
             Nếu {endpoint} là server tự host không cần key thì bỏ qua thông báo này. \
             Phản hồi của server: {body}",
            env = api_key_env,
        ));
    }
    LlmError::HttpStatus { status, body }
}

fn endpoint_url(base_url: Option<&str>) -> String {
    let base = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

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

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        self.chat_with_model(req, &self.model.clone()).await
    }

    async fn chat_stream(&self, req: ChatRequest<'_>) -> Result<LlmStream, LlmError> {
        self.chat_stream_with_model(req, &self.model.clone()).await
    }

    async fn chat_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmResponse, LlmError> {
        ensure_non_empty(&req)?;
        let body = build_request_body(req, model, false);
        let endpoint = self.endpoint.clone();
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let api_key_env = self.api_key_env.clone();

        retry::retry_with_backoff(retry::BASE_DELAY, retry::MAX_RETRIES, move || {
            let body = body.clone();
            let api_key = api_key.clone();
            let endpoint = endpoint.clone();
            let api_key_env = api_key_env.clone();
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
                    return Err(status_error(
                        &endpoint,
                        api_key.is_none(),
                        &api_key_env,
                        status.as_u16(),
                        error_body(&text),
                    ));
                }
                parse_response(&text)
            }
        })
        .await
    }

    async fn chat_stream_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmStream, LlmError> {
        let mut body = build_request_body(req, model, true);
        body["stream_options"] = json!({ "include_usage": true });
        let response = self.open_stream(body).await?;
        Ok(Box::pin(decode_sse(
            response,
            OpenAiStreamParser {
                usage: None,
                stop: None,
                done: false,
            },
        )))
    }

    fn name(&self) -> &'static str {
        "openai_compat"
    }
}
