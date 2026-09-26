//! Tool `marketing_publish` — đăng nội dung thật ra nền tảng (M24).
//!
//! # Ba bất biến, tầng ngoài phải đúng trước
//!
//! 1. **Luôn `Dangerous`, khai cứng trong code** — không đọc mức rủi ro từ cấu hình.
//!    `Policy::decide` trả `allow_in_session = false` cho mức này (mục 7.2), nên không có
//!    "cho phép trong phiên" dù cấu hình có gì. Đăng bài là hành động **không hoàn tác**:
//!    xoá bài cũ không xoá được hậu quả (đã gửi đi, đã được thấy).
//! 2. **Chỉ khi đã cấu hình đủ** mới gọi mạng; thiếu endpoint/credential ⇒ chế độ stub
//!    trả thông báo rõ, tuyệt đối không gửi gì đi (như M22a).
//! 3. **Credential riêng** — `Config::validate_marketing` từ chối trùng với key LLM /
//!    search / Telegram, vì quyền "chỉ post" phải là credential riêng (D15.3).

use std::sync::Arc;

use beanagent_security::{SafeHttpClient, SsrfError};
use beanagent_tools::{Tool, ToolCtx, ToolError};
use beanagent_types::config::{MARKETING_PUBLISH_TAG, MarketingConfig};
use beanagent_types::{Risk, ToolSpec};
use schemars::JsonSchema;
use secrecy::SecretString;
use serde::Deserialize;

/// Tham số của `marketing_publish`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarketingPublishParams {
    /// Nội dung cần đăng (văn bản thuần).
    pub text: String,
    /// Tiêu đề/bài viết, tuỳ nền tảng (bỏ trống nếu không dùng).
    #[serde(default)]
    pub title: Option<String>,
}

/// Trần độ dài nội dung đăng (ký tự) — nhiều nền tảng chặn bài quá dài.
const MAX_TEXT_CHARS: usize = 3_000;

/// Lỗi khi đăng nội dung.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// Chưa cấu hình endpoint/credential — tool ở **chế độ stub**, không gọi mạng.
    #[error("chưa cấu hình [marketing].base_url hoặc biến credential")]
    NotConfigured,
    /// Lỗi HTTP/SSRF đã lọc (không chứa token).
    #[error("{0}")]
    Http(#[from] SsrfError),
    /// Response không phải UTF-8.
    #[error("response publish không phải UTF-8")]
    NotUtf8,
}

/// Thông báo khi chưa cấu hình — nói rõ **chưa có gì được đăng**.
const STUB_NOTICE: &str = "\
CHƯA CẤU HÌNH: chưa có gì được đăng cả. Cần đặt [marketing].base_url và biến môi trường \
[marketing].api_key_env (credential RIÊNG, quyền chỉ post) trong BeanAgent.toml. \
Muốn chỉ soạn nội dung mà chưa đăng, hãy dùng `marketing_draft`.";

/// Client đăng nội dung: `POST base_url` với body JSON và `Authorization: Bearer`.
pub struct PublishClient {
    base_url: Option<String>,
    text_field: String,
    key: Option<SecretString>,
    /// `None` khi chưa đủ cấu hình ⇒ không dựng client, không gọi mạng.
    http: Option<SafeHttpClient>,
}

impl std::fmt::Debug for PublishClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Không in key, cũng không in endpoint đầy đủ (có thể chứa id tài khoản).
        f.debug_struct("PublishClient")
            .field("configured", &self.is_configured())
            .finish_non_exhaustive()
    }
}

impl PublishClient {
    /// Dựng client từ cấu hình + credential đã đọc từ biến môi trường.
    #[must_use]
    pub fn new(config: &MarketingConfig, key: Option<SecretString>) -> Self {
        let base_url = config
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned);
        let text_field = config.text_field.trim().to_string();
        // Chỉ dựng HTTP client khi thực sự gọi được; lỗi dựng client không làm hỏng lúc
        // khởi động — tool sẽ báo chưa cấu hình.
        let http = if base_url.is_some() && key.is_some() {
            match SafeHttpClient::new() {
                Ok(client) => Some(client),
                Err(error) => {
                    tracing::warn!(%error, "không dựng được HTTP client cho marketing; tool sẽ chạy ở chế độ stub");
                    None
                }
            }
        } else {
            None
        };
        Self {
            base_url,
            text_field,
            key,
            http,
        }
    }

    /// Đã đủ cấu hình để đăng thật chưa?
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.http.is_some()
    }

    /// Đăng nội dung; trả phản hồi của nền tảng để model biết đã thành công hay chưa.
    ///
    /// # Errors
    /// [`PublishError`] — chưa cấu hình, lỗi HTTP/SSRF, hoặc body không phải UTF-8.
    pub async fn publish(&self, params: &MarketingPublishParams) -> Result<String, PublishError> {
        let (Some(http), Some(key)) = (self.http.as_ref(), self.key.as_ref()) else {
            return Err(PublishError::NotConfigured);
        };
        let url = self.base_url.as_deref().unwrap_or_default();
        let mut body = serde_json::Map::new();
        body.insert(
            self.text_field.clone(),
            serde_json::Value::String(params.text.clone()),
        );
        if let Some(title) = params.title.as_deref().filter(|t| !t.trim().is_empty()) {
            body.insert(
                "title".to_string(),
                serde_json::Value::String(title.to_string()),
            );
        }
        let page = http
            .post_bearer(url, &serde_json::Value::Object(body), key)
            .await?;
        String::from_utf8(page.body).map_err(|_| PublishError::NotUtf8)
    }
}

/// Dựng tool `marketing_publish`.
#[must_use]
pub fn marketing_publish(client: PublishClient) -> Arc<dyn Tool> {
    let configured = client.is_configured();
    let description = if configured {
        "Đăng nội dung lên nền tảng marketing đã cấu hình. HÀNH ĐỘNG KHÔNG HOÀN TÁC — \
         luôn cần xác nhận người dùng trước khi chạy."
    } else {
        "Đăng nội dung lên nền tảng marketing. HIỆN CHƯA CẤU HÌNH — cần đặt \
         [marketing].base_url và biến credential riêng trong BeanAgent.toml."
    };
    let spec = ToolSpec::new(
        "marketing_publish",
        description,
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Nội dung cần đăng (văn bản thuần)."
                },
                "title": {
                    "type": "string",
                    "description": "Tiêu đề/bài viết, tuỳ nền tảng. Bỏ trống nếu không dùng."
                }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
    );
    Arc::new(MarketingPublishTool { client, spec })
}

struct MarketingPublishTool {
    client: PublishClient,
    spec: ToolSpec,
}

#[async_trait::async_trait]
impl Tool for MarketingPublishTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    /// **Cứng `Dangerous`, không đọc từ cấu hình** (M24 yêu cầu "kiểm tra cứng ở code").
    ///
    /// Hệ quả: `Policy::decide` luôn trả `NeedsConfirm { allow_in_session: false }` ⇒
    /// không có tuỳ chọn "cho phép trong phiên" (mục 7.2), kể cả khi người dùng đã duyệt
    /// tool này trước đó. Đây là chốt chặn duy nhất trước hành động không hoàn tác.
    fn risk(&self, _args: &serde_json::Value) -> Risk {
        Risk::Dangerous
    }

    /// (M24) Chỉ role giữ tag `marketing-publish` mới thấy/cọp tool này.
    fn required_tags(&self) -> Vec<&str> {
        vec![MARKETING_PUBLISH_TAG]
    }

    /// Phản hồi của nền tảng là dữ liệu **ngoài lõi** (mục 15.4) — bọc untrusted, kể cả
    /// ở nhánh lỗi, vì response có thể chứa chỉ dẫn do server bên thứ ba sinh ra.
    fn marks_untrusted(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params: MarketingPublishParams =
            beanagent_tools::typed::deserialize_params(args, &self.spec.parameters)
                .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        if params.text.trim().is_empty() {
            return Err(ToolError::InvalidArgs("nội dung rỗng".into()));
        }
        if params.text.chars().count() > MAX_TEXT_CHARS {
            return Err(ToolError::InvalidArgs(format!(
                "nội dung dài quá {MAX_TEXT_CHARS} ký tự — nhiều nền tảng chặn bài quá dài"
            )));
        }
        match self.client.publish(&params).await {
            Ok(response) => Ok(beanagent_tools::wrap_bounded(
                &response,
                beanagent_tools::MAX_WRAPPED_OUTPUT_CHARS,
            )),
            // Chế độ stub: KHÔNG gọi mạng, nói rõ để model không tưởng đã đăng thành công.
            Err(PublishError::NotConfigured) => Ok(STUB_NOTICE.to_string()),
            Err(error) => {
                // Bật cờ untrusted kể cả khi lỗi: đã chạm vào nguồn ngoài lõi (mục 15.4).
                ctx.untrusted_seen
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Err(ToolError::Io(beanagent_tools::wrap_untrusted(
                    &error.to_string(),
                )))
            }
        }
    }
}
