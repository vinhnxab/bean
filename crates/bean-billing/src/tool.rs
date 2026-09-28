//! Tool `billing_read_cost` — đọc chi phí cloud, read-only (Plan.md M22a).

use std::sync::Arc;

use bean_security::{SafeHttpClient, SsrfError};
use bean_tools::{Tool, ToolCtx, ToolError};
use bean_types::config::{BILLING_TAG, BillingConfig};
use bean_types::{Risk, ToolSpec};
use schemars::JsonSchema;
use secrecy::SecretString;
use serde::Deserialize;

/// Tham số của `billing_read_cost`.
///
/// Doc comment của struct/field chính là `description` gửi cho model (mục 7.1) nên phải nói rõ
/// khi nào dùng — model không được tự bịa kỳ nguyện.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BillingCostParams {
    /// Chu kỳ cần xem, ví dụ `7d`, `30d`, `mtd`. Bỏ trống thì dùng chu kỳ mặc định của provider.
    #[serde(default)]
    pub period: Option<String>,
    /// Ghi chú tự do cho người dùng (được gửi kèm để người đọc report hiểu ngữ cảnh).
    #[serde(default)]
    pub note: Option<String>,
}

/// Lỗi khi đọc chi phí.
#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    /// Endpoint chưa cấu hình — tool ở **chế độ stub**, không gọi mạng.
    #[error("chưa cấu hình endpoint billing")]
    NotConfigured,
    /// Lỗi HTTP/SSRF đã lọc (không chứa token).
    #[error("{0}")]
    Http(#[from] SsrfError),
    /// Body không phải UTF-8.
    #[error("response billing không phải UTF-8")]
    NotUtf8,
}

/// Nguồn dữ liệu billing mà tool gọi.
///
/// Tách thành trait để test **không cần network**: test dùng bản giả trả sẵn JSON, còn bản
/// thật gọi HTTP qua [`SafeHttpClient`].
pub trait BillingSource: Send + Sync {
    /// Trả JSON chi phí đã serialize, hoặc lỗi đã kiểm soát.
    fn fetch_cost<'a>(
        &'a self,
        params: &'a BillingCostParams,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, BillingError>> + Send + 'a>,
    >;
}

impl std::fmt::Debug for dyn BillingSource + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BillingSource")
    }
}

/// Nguồn billing thật: `GET base_url` với `Authorization: Bearer <key>`.
///
/// **Chỉ đọc**: không có đường vòng nào ghi/thay đổi hạ tầng hay chi phí.
pub struct BillingClient {
    base_url: Option<String>,
    query_suffix: Option<String>,
    key: Option<SecretString>,
    /// `None` khi chưa đủ cấu hình ⇒ không dựng client, không gọi mạng.
    http: Option<SafeHttpClient>,
}

impl std::fmt::Debug for BillingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Không in key, cũng không in endpoint đầy đủ (có thể chứa tenant/subscription id).
        f.debug_struct("BillingClient")
            .field("configured", &self.is_configured())
            .finish_non_exhaustive()
    }
}

impl BillingClient {
    /// Dựng client từ cấu hình + credential đã đọc từ biến môi trường.
    ///
    /// Chưa đủ `base_url` **hoặc** chưa có credential ⇒ **chế độ stub**: không gọi mạng, tool
    /// trả thông báo hướng dẫn cấu hình (D13.3).
    #[must_use]
    pub fn new(config: &BillingConfig, key: Option<SecretString>) -> Self {
        let base_url = config
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned);
        let query_suffix = config
            .query_suffix
            .as_deref()
            .map(str::trim)
            .filter(|suffix| !suffix.is_empty())
            .map(str::to_owned);
        // Chỉ dựng HTTP client khi thực sự gọi được; lỗi dựng client không làm hỏng lúc khởi
        // động — tool sẽ báo chưa cấu hình.
        let http = if base_url.is_some() && key.is_some() {
            match SafeHttpClient::new() {
                Ok(client) => Some(client),
                Err(error) => {
                    tracing::warn!(%error, "không dựng được HTTP client cho billing; tool sẽ chạy ở chế độ stub");
                    None
                }
            }
        } else {
            None
        };
        Self {
            base_url,
            query_suffix,
            key,
            http,
        }
    }

    /// `true` khi đủ cấu hình + credential để gọi thật.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.http.is_some() && self.key.is_some()
    }

    /// Ghép URL cuối cùng từ `base_url` + `period` + `query_suffix`.
    fn build_url(&self, period: Option<&str>) -> String {
        let mut url = self.base_url.clone().unwrap_or_default();
        let mut push = |fragment: &str| {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(fragment);
        };
        if let Some(period) = period {
            push("period=");
            push(period);
        }
        if let Some(suffix) = &self.query_suffix {
            push(suffix.trim_start_matches(['?', '&']));
        }
        url
    }
}

impl BillingSource for BillingClient {
    fn fetch_cost<'a>(
        &'a self,
        params: &'a BillingCostParams,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, BillingError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let (Some(http), Some(key)) = (self.http.as_ref(), self.key.as_ref()) else {
                return Err(BillingError::NotConfigured);
            };
            let period = params
                .period
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty());
            let page = http.fetch_bearer(&self.build_url(period), key).await?;
            String::from_utf8(page.body).map_err(|_| BillingError::NotUtf8)
        })
    }
}

/// Dựng tool `billing_read_cost` cho một nguồn billing bất kỳ.
#[must_use]
pub fn billing_read_cost(source: Arc<dyn BillingSource>, configured: bool) -> Arc<dyn Tool> {
    let description = if configured {
        "Đọc chi phí hạ tầng cloud từ hệ thống billing của công ty (chỉ đọc, không thay đổi gì)."
    } else {
        "Đọc chi phí hạ tầng cloud. HIỆN CHƯA CẤU HÌNH — cần đặt [billing].base_url và biến môi \
         trường [billing].api_key_env trong bean.toml."
    };
    let spec = ToolSpec::new(
        "billing_read_cost",
        description,
        serde_json::json!({
            "type": "object",
            "properties": {
                "period": {
                    "type": "string",
                    "description": "Chu kỳ cần xem, ví dụ 7d, 30d, mtd. Bỏ trống = mặc định provider."
                },
                "note": {
                    "type": "string",
                    "description": "Ghi chú tự do kèm theo để người đọc report hiểu ngữ cảnh."
                }
            },
            "additionalProperties": false
        }),
    );
    Arc::new(BillingCostTool { source, spec })
}

/// Tool đã đóng gói schema + handler.
struct BillingCostTool {
    source: Arc<dyn BillingSource>,
    spec: ToolSpec,
}

#[async_trait::async_trait]
impl Tool for BillingCostTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        // Chỉ đọc dữ liệu tốn tiền: `Safe` là hợp lý. Rủi ro nằm ở *rò dữ liệu tài chính*,
        // đã xử lý bằng RBAC tag thay vì bằng xác nhận thủ công.
        Risk::Safe
    }

    /// (M22a) Domain tài chính: chỉ role giữ tag `billing-read` mới thấy/cọp tool này.
    fn required_tags(&self) -> Vec<&str> {
        vec![BILLING_TAG]
    }

    /// Dữ liệu chi phí là dữ liệu **ngoài lõi** (mục 15.4) — bọc `<untrusted_content>` và
    /// bật cờ `untrusted_seen` ngay cả ở nhánh lỗi.
    fn marks_untrusted(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let params: BillingCostParams =
            bean_tools::typed::deserialize_params(args, &self.spec.parameters)
                .map_err(|err| ToolError::InvalidArgs(err.to_string()))?;
        let source = self.source.clone();
        let result = source.fetch_cost(&params).await;
        match result {
            Ok(json) => Ok(bean_tools::wrap_bounded(
                &json,
                bean_tools::MAX_WRAPPED_OUTPUT_CHARS,
            )),
            // Chế độ stub: KHÔNG gọi mạng, trả lời rõ để model không tưởng đã đọc được số thật.
            Err(BillingError::NotConfigured) => Ok(STUB_NOTICE.to_string()),
            Err(error) => {
                ctx.untrusted_seen
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Err(ToolError::Io(bean_tools::wrap_untrusted(
                    &error.to_string(),
                )))
            }
        }
    }
}

/// Thông báo chế độ stub — nói rõ đây là kết quả dự kiến, không phải lỗi hệ thống.
const STUB_NOTICE: &str = "[billing] Chưa cấu hình endpoint billing: đặt `[billing].base_url` \
và biến môi trường `[billing].api_key_env` trong bean.toml. Hiện KHÔNG gọi được nhà cung cấp \
cloud nào — đây là kết quả dự kiến, không phải lỗi.";

/// Nguồn giả trả sẵn JSON — cho test, không gọi mạng.
#[must_use]
pub fn stub_source(json: &str) -> Arc<dyn BillingSource> {
    Arc::new(StubSource {
        json: json.to_string(),
    })
}

struct StubSource {
    json: String,
}

impl BillingSource for StubSource {
    fn fetch_cost<'a>(
        &'a self,
        _params: &'a BillingCostParams,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, BillingError>> + Send + 'a>,
    > {
        let json = self.json.clone();
        Box::pin(async move { Ok(json) })
    }
}
