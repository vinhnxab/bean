//! Chính sách retry/backoff + factory + an toàn secret (agents.md mục 20 — M2).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use bean_llm::{ChatRequest, LlmError, MAX_RETRIES, build_provider, retry_with_backoff};
use bean_types::Message;
use bean_types::config::{LlmConfig, LlmProviderKind};

// ===== retry_with_backoff (unit, delay 1ms để test nhanh) =====

#[tokio::test]
async fn retries_transient_errors_then_succeeds() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let result: Result<&str, LlmError> =
        retry_with_backoff(Duration::from_millis(1), 3, move || {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(LlmError::Transport("mạng chập chờn".to_string()))
                } else {
                    Ok("thành công")
                }
            }
        })
        .await;
    assert_eq!(result.unwrap(), "thành công");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn non_retryable_error_returns_immediately() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let result: Result<&str, LlmError> =
        retry_with_backoff(Duration::from_millis(1), 3, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Err(LlmError::Decode("hỏng luôn".to_string())) }
        })
        .await;
    assert!(matches!(result.unwrap_err(), LlmError::Decode(_)));
    assert_eq!(attempts.load(Ordering::SeqCst), 1, "4xx không được retry");
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    type MakeError = fn() -> LlmError;
    let cases: [(&str, MakeError); 2] = [
        ("transport", || {
            LlmError::Transport("xuống cả hai".to_string())
        }),
        ("rate_limited", || LlmError::RateLimited {
            retry_after: None,
        }),
    ];
    for (name, make_err) in cases {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&attempts);
        let result: Result<&str, LlmError> =
            retry_with_backoff(Duration::from_millis(1), 2, move || {
                counter.fetch_add(1, Ordering::SeqCst);
                let err = make_err();
                async move { Err(err) }
            })
            .await;
        assert!(result.is_err(), "{name} phải lỗi");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "{name}: 1 lần đầu + 2 lần retry"
        );
    }
    let _ = MAX_RETRIES;
}

#[tokio::test]
async fn rate_limited_uses_retry_after_header() {
    // Có Retry-After → dùng đúng giá trị đó, KHÔNG dùng base_delay (đang là 10s).
    let start = std::time::Instant::now();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&attempts);
    let result: Result<&str, LlmError> =
        retry_with_backoff(Duration::from_secs(10), 3, move || {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Err(LlmError::RateLimited {
                        retry_after: Some(Duration::from_secs(1)),
                    })
                } else {
                    Ok("xong")
                }
            }
        })
        .await;
    assert_eq!(result.unwrap(), "xong");
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(900),
        "phải chờ Retry-After: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "không được dùng backoff 10s: {elapsed:?}"
    );
}

// ===== Retry ở mức HTTP (wiremock) =====

fn config(provider: LlmProviderKind, base_url: String) -> LlmConfig {
    LlmConfig {
        provider,
        model: "m-test".to_string(),
        allowed_models: vec![],
        api_key_env: "X_TEST_KEY".to_string(),
        base_url: Some(base_url),
        max_tokens: 64,
    }
}

fn chat_request() -> ChatRequest<'static> {
    ChatRequest {
        system: "",
        messages: Box::leak(Box::new([Message::user("x")])),
        tools: &[],
        max_tokens: 16,
    }
}

#[tokio::test]
async fn http_500_is_retried_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("lỗi server"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn",
            "usage": {}
        })))
        .mount(&server)
        .await;

    let cfg = config(LlmProviderKind::Anthropic, format!("{}/v1", server.uri()));
    let provider = build_provider(&cfg, Some(SecretString::from("k"))).unwrap();
    // Backoff thật: delay 1s sau lần 500 → test này chậm ~1s, chấp nhận (chứng minh có chờ).
    let start = std::time::Instant::now();
    let resp = provider.chat(chat_request()).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("ok"));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(
        start.elapsed() >= Duration::from_millis(900),
        "giữa 2 lần phải có backoff"
    );
}

#[tokio::test]
async fn http_400_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let cfg = config(LlmProviderKind::Anthropic, format!("{}/v1", server.uri()));
    let provider = build_provider(&cfg, Some(SecretString::from("k"))).unwrap();
    let err = provider.chat(chat_request()).await.unwrap_err();
    assert!(
        matches!(err, LlmError::HttpStatus { status: 400, .. }),
        "{err:?}"
    );
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "4xx phải dừng ngay, không retry"
    );
}

// ===== Factory =====

#[test]
fn factory_builds_anthropic_with_key() {
    let cfg = config(
        LlmProviderKind::Anthropic,
        "https://api.anthropic.com".to_string(),
    );
    let provider = build_provider(&cfg, Some(SecretString::from("k"))).unwrap();
    assert_eq!(provider.name(), "anthropic");
}

#[test]
fn factory_rejects_anthropic_without_key() {
    let cfg = config(
        LlmProviderKind::Anthropic,
        "https://api.anthropic.com".to_string(),
    );
    let err = build_provider(&cfg, None).unwrap_err();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
    // Lỗi phải nêu TÊN biến môi trường (không nêu giá trị).
    assert!(err.to_string().contains("X_TEST_KEY"), "{err}");
    // Key chuỗi rỗng coi như không có key.
    let err = build_provider(&cfg, Some(SecretString::from(""))).unwrap_err();
    assert!(matches!(err, LlmError::Config(_)), "{err:?}");
}

#[test]
fn factory_openai_compat_tolerates_missing_key_with_base_url() {
    let cfg = config(
        LlmProviderKind::OpenAiCompat,
        "http://127.0.0.1:11434/v1".to_string(),
    );
    let provider = build_provider(&cfg, None).unwrap();
    assert_eq!(provider.name(), "openai_compat");
    // Nhưng không có base_url và không có key → lỗi.
    let mut cfg = config(LlmProviderKind::OpenAiCompat, String::new());
    cfg.base_url = None;
    assert!(build_provider(&cfg, None).is_err());
}

// ===== An toàn secret (agents.md mục 15.6) =====

#[test]
fn debug_never_contains_api_key() {
    let provider = bean_llm::AnthropicProvider::new(
        "m",
        SecretString::from("KEY-ANHROPIC-BI-MAT-9f8e7d6c"),
        None,
    )
    .unwrap();
    let rendered = format!("{provider:?}");
    assert!(
        !rendered.contains("KEY-ANHROPIC-BI-MAT-9f8e7d6c"),
        "{rendered}"
    );
    assert!(rendered.contains("<redacted>"), "{rendered}");

    let provider = bean_llm::OpenAiCompatProvider::new(
        "m",
        Some(SecretString::from("KEY-OPENAI-BI-MAT-1234abcd")),
        None,
    )
    .unwrap();
    let rendered = format!("{provider:?}");
    assert!(
        !rendered.contains("KEY-OPENAI-BI-MAT-1234abcd"),
        "{rendered}"
    );
    assert!(rendered.contains("<redacted>"), "{rendered}");
}
