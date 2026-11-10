//! Failover giữa các provider (lộ trình "fallback provider" — thực tế đã gặp:
//! OpenRouter `:free` trả 503 và K17 flaky với model free).
//!
//! Provider chính đã tự retry 429/5xx/mạng/timeout tối đa 3 lần với backoff
//! ([`crate::retry_with_backoff`]). Khi vẫn hỏng, [`FailoverProvider`] thử lần
//! lượt các provider dự phòng theo `[llm].fallback_profiles`. 401/403 (key hỏng
//! hoặc hết hạn) cũng failover — đó là lý do người dùng khai báo profile dự
//! phòng. Lỗi khác (Decode, Config…) **không** failover: đó là vấn đề của
//! request/model hiện tại, chuyển provider chỉ che bug và làm mất tín hiệu.
//!
//! Streaming: chỉ failover khi [`LlmProvider::chat_stream`] trả `Err` **trước**
//! khi có delta nào (chưa có gì gửi đi nên không mất dữ liệu). Lỗi giữa dòng
//! đã được trả cho caller thì không thể tua lại — không failover.
//!
//! Model override (`chat_with_model`): áp dụng **chỉ cho provider chính**. Fallback
//! luôn chạy model riêng của profile nó — profile dự phòng là một định nghĩa hoàn
//! chỉnh (provider + model), ép model của primary vào fallback là sai (model free
//! của OpenRouter không tồn tại trên NVIDIA ⇒ 404). Hệ quả chấp nhận được: compaction
//!/reflection với `small_model` set mà failover thì chạy trên model của fallback
//! (có thể đắt hơn) — đúng vẫn hơn rẻ-sai.

use std::sync::Arc;

use async_trait::async_trait;
use bean_types::LlmResponse;

use crate::LlmProvider;
use crate::error::LlmError;
use crate::{ChatRequest, LlmStream};

/// Provider chính + chuỗi dự phòng; failover theo [`should_failover`].
#[derive(Debug)]
pub struct FailoverProvider {
    primary: Arc<dyn LlmProvider>,
    fallbacks: Vec<Arc<dyn LlmProvider>>,
}

impl FailoverProvider {
    /// Gói `primary` với chuỗi dự phòng. Chuỗi rỗng ⇒ trả nguyên `primary`
    /// (không thêm một lớp bọc vô ích vào `name()`/log).
    #[must_use]
    pub fn attach(
        primary: Arc<dyn LlmProvider>,
        fallbacks: Vec<Arc<dyn LlmProvider>>,
    ) -> Arc<dyn LlmProvider> {
        if fallbacks.is_empty() {
            return primary;
        }
        Arc::new(Self { primary, fallbacks })
    }

    /// Số provider dự phòng (test/trạng thái).
    #[must_use]
    pub fn fallback_count(&self) -> usize {
        self.fallbacks.len()
    }
}

/// Lỗi cho phép chuyển sang provider dự phòng.
///
/// * `is_retryable()` đã hết retry trong provider hiện tại (429/5xx/mạng/timeout);
/// * 401/403 — key hỏng/hết hạn/sai quyền;
/// * `Cancelled` **không** failover (người dùng muốn dừng là dừng);
/// * 4xx khác và Decode/Config không failover (xem doc module).
#[must_use]
pub fn should_failover(error: &LlmError) -> bool {
    if error.is_retryable() {
        return true;
    }
    matches!(
        error,
        LlmError::HttpStatus {
            status: 401 | 403,
            ..
        }
    )
}

#[async_trait]
impl LlmProvider for FailoverProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        let mut last: Option<LlmError> = None;
        let total = self.fallbacks.len() + 1;
        for (idx, provider) in std::iter::once(self.primary.as_ref())
            .chain(self.fallbacks.iter().map(|p| p.as_ref()))
            .enumerate()
        {
            match provider.chat(req).await {
                Ok(response) => {
                    if idx > 0 {
                        tracing::warn!(
                            provider = provider.name(),
                            attempt = idx + 1,
                            total,
                            "failover: provider dự phòng trả lời thành công"
                        );
                    }
                    return Ok(response);
                }
                Err(error) => {
                    if !should_failover(&error) {
                        return Err(error);
                    }
                    tracing::warn!(
                        provider = provider.name(),
                        error = %error,
                        "provider lỗi tạm thời — chuyển sang provider dự phòng"
                    );
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| LlmError::Internal("không có provider nào để gọi".into())))
    }

    async fn chat_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmResponse, LlmError> {
        let mut last: Option<LlmError> = None;
        for (idx, provider) in std::iter::once(&self.primary)
            .chain(self.fallbacks.iter())
            .enumerate()
        {
            // Override chỉ áp cho primary; fallback chạy model riêng của profile nó.
            let result = if idx == 0 {
                provider.chat_with_model(req, model).await
            } else {
                provider.chat(req).await
            };
            match result {
                Ok(response) => return Ok(response),
                Err(error) => {
                    if !should_failover(&error) {
                        return Err(error);
                    }
                    tracing::warn!(
                        provider = provider.name(),
                        error = %error,
                        "provider lỗi tạm thời (model override) — chuyển sang provider dự phòng"
                    );
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| LlmError::Internal("không có provider nào để gọi".into())))
    }

    async fn chat_stream(&self, req: ChatRequest<'_>) -> Result<LlmStream, LlmError> {
        let mut last: Option<LlmError> = None;
        for provider in std::iter::once(&self.primary).chain(self.fallbacks.iter()) {
            match provider.chat_stream(req).await {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    if !should_failover(&error) {
                        return Err(error);
                    }
                    tracing::warn!(
                        provider = provider.name(),
                        error = %error,
                        "provider lỗi tạm thời (streaming) — chuyển sang provider dự phòng"
                    );
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| LlmError::Internal("không có provider nào để gọi".into())))
    }

    async fn chat_stream_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmStream, LlmError> {
        let mut last: Option<LlmError> = None;
        for (idx, provider) in std::iter::once(&self.primary)
            .chain(self.fallbacks.iter())
            .enumerate()
        {
            // Override chỉ áp cho primary; fallback chạy model riêng của profile nó.
            let result = if idx == 0 {
                provider.chat_stream_with_model(req, model).await
            } else {
                provider.chat_stream(req).await
            };
            match result {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    if !should_failover(&error) {
                        return Err(error);
                    }
                    tracing::warn!(
                        provider = provider.name(),
                        error = %error,
                        "provider lỗi tạm thời (streaming + model override) — chuyển sang provider dự phòng"
                    );
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| LlmError::Internal("không có provider nào để gọi".into())))
    }

    fn name(&self) -> &'static str {
        "failover"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use bean_types::{Message, StopReason, Usage};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    enum Scripted {
        /// Luôn trả lỗi retryable (503) — failover.
        Unavailable(&'static str),
        /// Trả lỗi 401 — failover.
        Unauthorized(&'static str),
        /// Trả lỗi không failover (Decode) — giữ nguyên.
        Decode(&'static str),
        /// Ok sau N lần gọi (đếm theo provider).
        Ok(&'static str),
    }

    #[derive(Debug)]
    struct ScriptedProvider {
        script: Scripted,
        calls: AtomicUsize,
    }

    impl ScriptedProvider {
        fn scripted(script: Scripted) -> Arc<dyn LlmProvider> {
            Arc::new(Self {
                script,
                calls: AtomicUsize::new(0),
            })
        }
    }

    fn ok_response() -> LlmResponse {
        LlmResponse {
            text: Some("ok".into()),
            tool_calls: vec![],
            stop: StopReason::EndTurn,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn chat(&self, _req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.script {
                Scripted::Unavailable(name) => Err(LlmError::HttpStatus {
                    status: 503,
                    body: name.into(),
                }),
                Scripted::Unauthorized(name) => Err(LlmError::HttpStatus {
                    status: 401,
                    body: name.into(),
                }),
                Scripted::Decode(name) => Err(LlmError::Decode(name.into())),
                Scripted::Ok(name) => {
                    let _ = name;
                    Ok(ok_response())
                }
            }
        }
        fn name(&self) -> &'static str {
            "scripted"
        }
    }

    fn req() -> ChatRequest<'static> {
        // Leak cố ý trong test: tạo slice 'static cho vài chục lần gọi, không dọn
        // cũng không ảnh hưởng gì.
        let messages: &'static [Message] = Box::leak(Box::new([Message::user(String::from("hi"))]));
        ChatRequest {
            system: "",
            messages,
            tools: &[],
            max_tokens: 16,
        }
    }

    #[tokio::test]
    async fn failover_tries_fallback_on_5xx_and_reports_success() {
        let provider = FailoverProvider::attach(
            ScriptedProvider::scripted(Scripted::Unavailable("chính")),
            vec![ScriptedProvider::scripted(Scripted::Ok("dự phòng"))],
        );
        assert_eq!(provider.name(), "failover");
        let response = provider
            .chat(req())
            .await
            .expect("dự phòng phải thành công");
        assert_eq!(response.text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn failover_on_401_broken_key() {
        let provider = FailoverProvider::attach(
            ScriptedProvider::scripted(Scripted::Unauthorized("chính")),
            vec![ScriptedProvider::scripted(Scripted::Ok("dự phòng"))],
        );
        assert!(provider.chat(req()).await.is_ok(), "401 phải failover");
    }

    #[tokio::test]
    async fn non_failoverable_error_passes_through_immediately() {
        let provider = FailoverProvider::attach(
            ScriptedProvider::scripted(Scripted::Decode("JSON hỏng")),
            vec![ScriptedProvider::scripted(Scripted::Ok("dự phòng"))],
        );
        let error = provider
            .chat(req())
            .await
            .expect_err("Decode không được failover");
        assert!(matches!(error, LlmError::Decode(_)));
    }

    #[tokio::test]
    async fn all_candidates_failing_returns_last_error() {
        let provider = FailoverProvider::attach(
            ScriptedProvider::scripted(Scripted::Unavailable("chính")),
            vec![ScriptedProvider::scripted(Scripted::Unavailable(
                "dự phòng",
            ))],
        );
        let error = provider
            .chat(req())
            .await
            .expect_err("hết đường phải trả lỗi");
        assert!(matches!(error, LlmError::HttpStatus { status: 503, .. }));
    }

    #[tokio::test]
    async fn empty_fallbacks_returns_primary_unwrapped() {
        let provider =
            FailoverProvider::attach(ScriptedProvider::scripted(Scripted::Ok("chính")), vec![]);
        assert_eq!(
            provider.name(),
            "scripted",
            "không bọc khi không có dự phòng"
        );
        assert!(provider.chat(req()).await.is_ok());
    }

    #[test]
    fn cancelled_never_fails_over() {
        assert!(!should_failover(&LlmError::Cancelled));
        assert!(should_failover(&LlmError::Transport("mạng đứt".into())));
        assert!(should_failover(&LlmError::Timeout));
    }

    /// Ghi lại đường gọi: `chat` hay `chat_with_model(model)` — dùng để khẳng định
    /// fallback KHÔNG nhận model override của primary.
    #[derive(Debug)]
    struct ModelPathRecorder {
        /// `Some(status)` = luôn trả `HttpStatus{status}`; `None` = trả Ok.
        fail_with: Option<u16>,
        path: Mutex<Vec<String>>,
    }

    impl ModelPathRecorder {
        fn result(&self) -> Result<LlmResponse, LlmError> {
            match self.fail_with {
                Some(status) => Err(LlmError::HttpStatus {
                    status,
                    body: "chết".into(),
                }),
                None => Ok(ok_response()),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for ModelPathRecorder {
        async fn chat(&self, _req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
            self.path.lock().unwrap().push("chat".into());
            self.result()
        }

        async fn chat_with_model(
            &self,
            _req: ChatRequest<'_>,
            model: &str,
        ) -> Result<LlmResponse, LlmError> {
            self.path
                .lock()
                .unwrap()
                .push(format!("with_model:{model}"));
            self.result()
        }

        fn name(&self) -> &'static str {
            "recorder"
        }
    }

    #[tokio::test]
    async fn fallback_ignores_model_override() {
        let primary = Arc::new(ModelPathRecorder {
            fail_with: Some(503),
            path: Mutex::new(vec![]),
        });
        let fallback = Arc::new(ModelPathRecorder {
            fail_with: None,
            path: Mutex::new(vec![]),
        });
        let provider = FailoverProvider::attach(primary.clone(), vec![fallback.clone()]);
        let response = provider
            .chat_with_model(req(), "model-của-chính")
            .await
            .expect("fallback phải thành công");
        assert_eq!(response.text.as_deref(), Some("ok"));
        assert_eq!(
            *primary.path.lock().unwrap(),
            ["with_model:model-của-chính"]
        );
        assert_eq!(
            *fallback.path.lock().unwrap(),
            ["chat"],
            "fallback phải chạy model riêng của profile nó"
        );
    }

    #[tokio::test]
    async fn chat_stream_fails_over_before_first_delta() {
        let provider = FailoverProvider::attach(
            ScriptedProvider::scripted(Scripted::Unavailable("chính")),
            vec![ScriptedProvider::scripted(Scripted::Ok("dự phòng"))],
        );
        let stream = provider.chat_stream(req()).await.expect("stream dự phòng");
        use futures_util::StreamExt;
        futures_util::pin_mut!(stream);
        let items: Vec<_> = stream.collect().await;
        assert!(items.iter().any(|item| item.is_ok()));
    }
}
