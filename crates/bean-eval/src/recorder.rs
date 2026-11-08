//! Proxy bọc [`LlmProvider`] để **đo** mà không đổi hành vi: gom usage của mọi
//! lượt gọi (kể cả compaction và reflection đi qua `chat_with_model`) và đếm
//! số bước — hai số liệu cốt lõi của metrics.
//!
//! DIP: runner phụ thuộc `Arc<dyn LlmProvider>`; proxy cắm vào giữa mà không
//! cần sửa Router hay provider thật.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bean_llm::{ChatRequest, LlmError, LlmProvider};
use bean_types::llm::{LlmResponse, Usage};

/// Tích luỹ usage qua các lượt gọi.
#[derive(Debug, Default)]
struct UsageAccumulator {
    input_tokens: u64,
    output_tokens: u64,
}

/// Provider ghi đè: ủy thác mọi thứ cho provider bên trong, ghi nhận usage.
pub struct UsageRecorder {
    inner: Arc<dyn LlmProvider>,
    usage: Mutex<UsageAccumulator>,
    calls: AtomicUsize,
}

impl std::fmt::Debug for UsageRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Không `{:?}` provider bên trong: Debug của provider đã che secret, nhưng
        // metrics không cần in nó — in tên là đủ (K30: không debug-in struct có key).
        f.debug_struct("UsageRecorder")
            .field("provider", &self.inner.name())
            .field("calls", &self.calls.load(Ordering::SeqCst))
            .finish()
    }
}

impl UsageRecorder {
    /// Bọc provider thật/giả.
    #[must_use]
    pub fn new(inner: Arc<dyn LlmProvider>) -> Self {
        Self {
            inner,
            usage: Mutex::new(UsageAccumulator::default()),
            calls: AtomicUsize::new(0),
        }
    }

    /// Tổng token vào của mọi lượt gọi.
    #[must_use]
    pub fn input_tokens(&self) -> u64 {
        self.usage.lock().map_or(0, |usage| usage.input_tokens)
    }

    /// Tổng token ra của mọi lượt gọi.
    #[must_use]
    pub fn output_tokens(&self) -> u64 {
        self.usage.lock().map_or(0, |usage| usage.output_tokens)
    }

    /// Số lượt gọi LLM (mỗi lượt ≈ một bước của vòng lặp + compaction/reflection).
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn record(&self, usage: &Usage) {
        if let Ok(mut accum) = self.usage.lock() {
            accum.input_tokens = accum
                .input_tokens
                .saturating_add(u64::from(usage.input_tokens));
            accum.output_tokens = accum
                .output_tokens
                .saturating_add(u64::from(usage.output_tokens));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl LlmProvider for UsageRecorder {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<LlmResponse, LlmError> {
        let response = self.inner.chat(req).await?;
        self.record(&response.usage);
        Ok(response)
    }

    async fn chat_with_model(
        &self,
        req: ChatRequest<'_>,
        model: &str,
    ) -> Result<LlmResponse, LlmError> {
        let response = self.inner.chat_with_model(req, model).await?;
        self.record(&response.usage);
        Ok(response)
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}
