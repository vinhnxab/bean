//! Nhóm tool **đọc** — `browser-read` (M26).
//!
//! Tag RBAC: `dev-read` **hoặc** `test-run` ⇒ cả Developer lẫn QA đều dùng được.
//! Mức rủi ro: `Safe` — đọc trang không làm thay đổi gì trên hệ thống.
//!
//! Nhưng `Safe` **không** có nghĩa là vô hại: mọi tool ở đây khai
//! `marks_untrusted() = true` vì nội dung trang là dữ liệu ngoài lõi (mục 15.4).
//! Hệ quả cụ thể: sau khi lượt đọc trang, mọi tool `Confirm` trở lên trong **cùng
//! lượt** đó sẽ hỏi lại và mất tuỳ chọn "cho phép trong phiên" — đây chính là
//! lớp phòng thủ chống prompt injection qua nội dung trang.

use std::borrow::Cow;
use std::sync::Arc;

use async_trait::async_trait;
use bean_tools::{Tool, ToolAccess, ToolCtx, ToolError, ToolOutput, wrap_bounded};
use bean_types::config::{DEV_READ_TAG, TEST_RUN_TAG};
use bean_types::{ImageBlock, Risk, ToolSpec};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::session::{LaunchError, SessionManager};

/// Tag RBAC cho nhóm đọc: Developer **hoặc** QA.
pub const READ_TAGS: [&str; 2] = [DEV_READ_TAG, TEST_RUN_TAG];

/// Chuyển lỗi khởi động Chrome thành `ToolError` để model tự đọc và sửa.
fn launch_error(error: LaunchError) -> ToolError {
    ToolError::Io(wrap_bounded(
        &format!("[browser] {error}"),
        bean_tools::untrusted::MAX_WRAPPED_OUTPUT_CHARS,
    ))
}

/// Bọc output đọc được trong `<untrusted_content>` (mục 15.4).
fn wrap(content: &str) -> String {
    wrap_bounded(content, bean_tools::untrusted::MAX_WRAPPED_OUTPUT_CHARS)
}

// ---------------------------------------------------------------------------
// browser_screenshot
// ---------------------------------------------------------------------------

/// Tham số của `browser_screenshot`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotParams {
    /// URL cần mở trước khi chụp. Bỏ trống thì chụp trang hiện tại.
    ///
    /// URL phải qua lớp kiểm tra của domain browser; origin nằm trong
    /// `[browser].allowed_origins` thì lớp này mới cho đi tới mạng nội bộ.
    #[serde(default)]
    pub url: Option<String>,
    /// Định dạng ảnh: `png` (mặc định) hoặc `jpeg`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Chụp màn hình trang hiện tại.
pub struct ScreenshotTool {
    session: Arc<SessionManager>,
    max_image_bytes: usize,
}

impl ScreenshotTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>, max_image_bytes: usize) -> Self {
        Self {
            session,
            max_image_bytes,
        }
    }
}

#[async_trait]
impl bean_tools::Tool for ScreenshotTool {
    fn spec(&self) -> ToolSpec {
        bean_tools::typed_spec::<ScreenshotParams>("browser_screenshot")
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn marks_untrusted(&self) -> bool {
        // Ảnh chụp trang là nội dung ngoài lõi: kẻ tấn công kiểm soát được trang
        // cũng kiểm soát được những gì hiện trong ảnh.
        true
    }

    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Borrowed(&READ_TAGS[..]),
            ..ToolAccess::default()
        }
    }

    async fn call(&self, _ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        // `call` không dùng cho tool ảnh; agent loop gọi `call_rich`. Trả lỗi rõ ràng
        // thay vì âm thầm rơi về text rỗng.
        let _ = args;
        Err(ToolError::Internal(
            "browser_screenshot trả ảnh, phải gọi qua `call_rich`".into(),
        ))
    }

    async fn call_rich(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
        let params: ScreenshotParams =
            bean_tools::deserialize_params(args, &self.spec().parameters)?;
        let (caption, image) = self.capture(ctx, &params).await?;
        // Bật cờ untrusted **giống hệt** `TypedTool::call` làm, để lớp phòng thủ
        // "cho phép trong phiên" bị vô hiệu khi lượt này đã đọc ảnh trang.
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(ToolOutput::Image { caption, image })
    }
}

impl ScreenshotTool {
    /// Chụp và dựng khối ảnh.
    async fn capture(
        &self,
        _ctx: &ToolCtx,
        params: &ScreenshotParams,
    ) -> Result<(String, ImageBlock), ToolError> {
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotParams;
        use sha2::{Digest, Sha256};

        let format = match params
            .format
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("png") => CaptureScreenshotFormat::Png,
            Some("jpeg") | Some("jpg") => CaptureScreenshotFormat::Jpeg,
            Some(other) => {
                return Err(ToolError::InvalidArgs(format!(
                    "format `{other}` không hợp lệ; chỉ nhận `png` hoặc `jpeg`"
                )));
            }
        };
        let media_type = match format {
            CaptureScreenshotFormat::Jpeg => "image/jpeg",
            _ => "image/png",
        };

        let page = self.session.page().await.map_err(launch_error)?;

        if let Some(url) = params.url.as_deref() {
            page.goto(url)
                .await
                .map_err(|e| ToolError::Io(wrap(&format!("không mở được URL: {e}"))))?;
        }

        let response = page
            .execute(CaptureScreenshotParams {
                format: Some(format),
                // Chỉ chụp viewport: ảnh toàn trang dài có thể vượt trần byte và
                // vô dụng với model.
                capture_beyond_viewport: Some(false),
                ..Default::default()
            })
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không chụp được ảnh: {e}"))))?;
        // `Binary` của crate CDP là **newtype base64** (`Binary(String)`), nên CDP
        // đã mã hoá sẵn. Ta dùng đúng chuỗi đó làm `data` — giải mã rồi mã hoá lại
        // chỉ tốn thêm CPU mà không đổi kết quả.
        let encoded: String = String::from(AsRef::<str>::as_ref(&response.data));
        let bytes_len = encoded.len();

        if encoded.len() > self.max_image_bytes {
            return Err(ToolError::Io(wrap(&format!(
                "ảnh {} KB vượt trần {} KB — thử chụp vùng nhỏ hơn hoặc tăng \
                 [browser].max_image_bytes",
                encoded.len() / 1024,
                self.max_image_bytes / 1024
            ))));
        }

        let url_now = page.url().await.unwrap_or(None).unwrap_or_default();
        // Hash trên chuỗi base64 mà CDP trả về: nó chính là byte ảnh, và ổn định
        // giữa các lần chụp cùng nội dung (base64 là mã hoá tiêu chuẩn, tất định).
        let digest = hex(&Sha256::digest(encoded.as_bytes()));
        let image = ImageBlock::new(media_type, encoded, digest).map_err(ToolError::Internal)?;
        let where_ = if url_now.is_empty() {
            String::new()
        } else {
            format!(" của {url_now}")
        };
        let caption = format!(
            "Ảnh chụp màn hình{where_} — {} KB, {media_type}",
            bytes_len / 1024
        );
        Ok((caption, image))
    }
}

/// SHA-256 ra hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// browser_console_logs
// ---------------------------------------------------------------------------

/// Tham số của `browser_console_logs`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConsoleLogsParams {
    /// URL cần mở trước khi đọc. Bỏ trống thì đọc trang hiện tại.
    #[serde(default)]
    pub url: Option<String>,
    /// Chỉ lấy log mức `error` thay vì mọi mức.
    #[serde(default)]
    pub errors_only: Option<bool>,
}

/// Đọc console log của trang (sự kiện `Log.entryAdded`).
pub struct ConsoleLogsTool {
    session: Arc<SessionManager>,
}

impl ConsoleLogsTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl bean_tools::Tool for ConsoleLogsTool {
    fn spec(&self) -> ToolSpec {
        bean_tools::typed_spec::<ConsoleLogsParams>("browser_console_logs")
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn marks_untrusted(&self) -> bool {
        // Thông điệp console do trang kiểm soát: có thể chứa cả chuỗi chỉ dẫn
        // độc hại nhắm vào chính model đang đọc log.
        true
    }

    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Borrowed(&READ_TAGS[..]),
            ..ToolAccess::default()
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        use futures_util::StreamExt as _;

        let params: ConsoleLogsParams =
            bean_tools::deserialize_params(args, &self.spec().parameters)?;
        let page = self.session.page().await.map_err(launch_error)?;
        page.enable_log()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không bật được Log domain: {e}"))))?;
        page.enable_runtime()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không bật được Runtime domain: {e}"))))?;

        let mut listener = page
            .event_listener::<chromiumoxide::cdp::browser_protocol::log::EventEntryAdded>()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không nghe được log: {e}"))))?;

        if let Some(url) = params.url.as_deref() {
            page.goto(url)
                .await
                .map_err(|e| ToolError::Io(wrap(&format!("không mở được URL: {e}"))))?;
        }

        // Thu thập trong một cửa sổ ngắn: console log là dữ liệu tức thời, chờ thêm
        // chỉ làm tool chậm mà không thêm thông tin.
        let deadline = tokio::time::Duration::from_millis(1_500);
        let mut entries: Vec<String> = Vec::new();
        let collect = async {
            while let Some(event) = listener.next().await {
                let entry = &event.entry;
                if params.errors_only.unwrap_or(false) && entry.level.as_ref() != "error" {
                    continue;
                }
                entries.push(format!(
                    "[{level}] {source}:{line} {text}",
                    level = entry.level.as_ref(),
                    source = entry.source.as_ref(),
                    line = entry.line_number.unwrap_or_default(),
                    text = entry.text,
                ));
                if entries.len() >= MAX_CONSOLE_ENTRIES {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(deadline, collect).await;

        let url_now = page.url().await.unwrap_or(None).unwrap_or_default();
        if entries.is_empty() {
            ctx.untrusted_seen
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Ok(wrap(&format!(
                "Không có console log nào trong 1.5 giây đầu tại {url_now}"
            )));
        }
        let body = format!("Console log tại {url_now}:\n{}", entries.join("\n"));
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(wrap(&body))
    }
}

/// Trần số dòng log thu được, để một trang lỗi nặng không làm nổ context.
const MAX_CONSOLE_ENTRIES: usize = 200;

// ---------------------------------------------------------------------------
// browser_network
// ---------------------------------------------------------------------------

/// Tham số của `browser_network`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkParams {
    /// URL cần mở trước khi đọc. Bỏ trống thì đọc trang hiện tại.
    #[serde(default)]
    pub url: Option<String>,
    /// Chỉ giữ request có URL chứa chuỗi này.
    #[serde(default)]
    pub url_filter: Option<String>,
    /// Số request tối đa trả về.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Đọc danh sách network request của trang.
pub struct NetworkTool {
    session: Arc<SessionManager>,
}

impl NetworkTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl bean_tools::Tool for NetworkTool {
    fn spec(&self) -> ToolSpec {
        bean_tools::typed_spec::<NetworkParams>("browser_network")
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn marks_untrusted(&self) -> bool {
        // URL request, header và tên host đều do trang kiểm soát; query string
        // còn có thể chứa token của ứng dụng đang test.
        true
    }

    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Borrowed(&READ_TAGS[..]),
            ..ToolAccess::default()
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        use futures_util::StreamExt as _;

        let params: NetworkParams = bean_tools::deserialize_params(args, &self.spec().parameters)?;
        let limit = params.limit.unwrap_or(100).clamp(1, 500) as usize;
        let page = self.session.page().await.map_err(launch_error)?;
        page.enable_log()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không bật được Log domain: {e}"))))?;

        let mut listener = page
            .event_listener::<chromiumoxide::cdp::browser_protocol::network::EventRequestWillBeSent>()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không nghe được Network domain: {e}"))))?;

        if let Some(url) = params.url.as_deref() {
            page.goto(url)
                .await
                .map_err(|e| ToolError::Io(wrap(&format!("không mở được URL: {e}"))))?;
        }

        let deadline = tokio::time::Duration::from_millis(2_000);
        let mut rows: Vec<String> = Vec::new();
        let collect = async {
            while let Some(event) = listener.next().await {
                let request = &event.request;
                if let Some(filter) = params.url_filter.as_deref()
                    && !request.url.contains(filter)
                {
                    continue;
                }
                rows.push(format!(
                    "{method} {url}",
                    method = request.method.as_str(),
                    url = request.url
                ));
                if rows.len() >= limit {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(deadline, collect).await;

        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if rows.is_empty() {
            return Ok(wrap(&format!(
                "Không thấy request nào khớp trong 2 giây (bộ lọc: {:?})",
                params.url_filter
            )));
        }
        Ok(wrap(&format!(
            "{} request:\n{}",
            rows.len(),
            rows.join("\n")
        )))
    }
}

// ---------------------------------------------------------------------------
// browser_performance_trace
// ---------------------------------------------------------------------------

/// Tham số của `browser_performance_trace`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerformanceParams {
    /// URL cần đo. Bắt buộc — trace chỉ có ý nghĩa khi có một lần tải trang thật.
    pub url: String,
}

/// Lấy metric hiệu năng thô của một lần tải trang.
///
/// **Không** upload lên CrUX hay bất kỳ dịch vụ phân tích nào: chỉ đọc metric từ
/// `Page.navigate` + `Performance.getMetrics`, dữ liệu ở lại máy.
pub struct PerformanceTool {
    session: Arc<SessionManager>,
}

impl PerformanceTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl bean_tools::Tool for PerformanceTool {
    fn spec(&self) -> ToolSpec {
        bean_tools::typed_spec::<PerformanceParams>("browser_performance_trace")
    }

    fn risk(&self, _args: &Value) -> Risk {
        Risk::Safe
    }

    fn marks_untrusted(&self) -> bool {
        true
    }

    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Borrowed(&READ_TAGS[..]),
            ..ToolAccess::default()
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        use chromiumoxide::cdp::browser_protocol::performance::GetMetricsParams;

        let params: PerformanceParams =
            bean_tools::deserialize_params(args, &self.spec().parameters)?;
        let page = self.session.page().await.map_err(launch_error)?;

        page.goto(&params.url)
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không mở được URL: {e}"))))?;
        // Chờ tải xong để metric phản ánh trang đã render, không phải trang trắng.
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            page.wait_for_navigation(),
        )
        .await;

        let metrics = page
            .execute(GetMetricsParams::default())
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không lấy được metric: {e}"))))?;

        // Chỉ giữ nhóm metric có ý nghĩa chẩn đoán; toàn bộ `Performance.getMetrics`
        // dài hàng trăm dòng và phần lớn là counter nội bộ.
        const INTERESTING: &[&str] = &[
            "Timestamp",
            "Documents",
            "Frames",
            "JSEventListeners",
            "Nodes",
            "LayoutCount",
            "RecalcStyleCount",
            "LayoutDuration",
            "RecalcStyleDuration",
            "ScriptDuration",
            "TaskDuration",
            "TaskOtherDuration",
            "ThreadTime",
            "ProcessTime",
        ];
        let rows: Vec<String> = metrics
            .metrics
            .iter()
            .filter(|m| INTERESTING.contains(&m.name.as_str()))
            .map(|m| format!("{} = {}", m.name, m.value))
            .collect();

        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(wrap(&format!(
            "Performance metrics của {}:\n{}",
            params.url,
            rows.join("\n")
        )))
    }
}
