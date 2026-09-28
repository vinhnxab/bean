//! Nhóm tool **hành động** — `browser-act` (M26).
//!
//! # Ranh giới RBAC: CHỈ role QA
//!
//! `required_tags = ["test-run"]` ⇒ role `qa` (giữ `dev-read` + `test-run`) gọi được,
//! còn role `developer` (giữ `dev-read` + `dev-write`) **không thấy và không gọi
//! được** bất kỳ tool nào ở đây. Đó là nguyên tắc four-eyes đã dùng cho
//! `run_shell`/`write_file`: người viết code không được tự kiểm chứng bằng hành
//! động trên trình duyệt.
//!
//! Chặn ở **hai** tầng, cùng một hàm `RolePermissions::allows`:
//!
//! * `specs_visible_to` — model không **thấy** tool ngoài quyền trong payload;
//! * `registry.allows` — chặn ở tầng thực thi khi `args` (JSON không tin cậy) bịa ra
//!   tên tool (mục 21.5).
//!
//! # Mức rủi ro theo origin
//!
//! * Origin **trong** `[browser].allowed_origins` ⇒ `Confirm` (có "cho phép trong
//!   phiên") — dùng hàng trăm lần mỗi phiên test sẽ không mệt.
//! * Origin **ngoài** whitelist ⇒ `Dangerous`, **không** session-wide (mục 7.2):
//!   `Policy` đã bảo đảm `Dangerous` không bao giờ được "cho phép trong phiên", kể
//!   cả khi `SessionPolicy` có sẵn mục.
//! * `browser_evaluate_script` **luôn** `Dangerous`: chạy JS tuỳ ý trên trang là
//!   tương đương thực thi mã, không gắn được khái niệm "origin được tin".
//!
//! # Vì sao `risk()` tra whitelist mà không cần Chrome
//!
//! `Tool::risk(&args)` là hàm **đồng bộ**, không có async, nên không thể hỏi CDP.
//! Thay vào đó mỗi tool hành động tra **origin trong tham số của chính lời gọi đó**
//! (`browser_navigate` có `url`), hoặc — với tool không mang URL — dùng
//! [`crate::session::SessionManager::current_origin`], giá trị được cập nhật mỗi
//! lần navigate. Cả hai đều **fail-closed**: không xác định được origin ⇒ `Dangerous`.

use std::sync::Arc;

use async_trait::async_trait;
use beanagent_tools::{ToolCtx, ToolError, wrap_bounded};
use beanagent_types::config::TEST_RUN_TAG;
use beanagent_types::{Risk, ToolSpec};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::guard::{BrowserGuard, GuardError};
use crate::origin::{OriginVerdict, OriginWhitelist};
use crate::session::{LaunchError, SessionManager};

/// Tag RBAC duy nhất cho nhóm hành động: **chỉ QA**.
pub const ACT_TAGS: [&str; 1] = [TEST_RUN_TAG];

/// Bọc output trong `<untrusted_content>` (mục 15.4).
fn wrap(content: &str) -> String {
    wrap_bounded(
        content,
        beanagent_tools::untrusted::MAX_WRAPPED_OUTPUT_CHARS,
    )
}

fn launch_error(error: LaunchError) -> ToolError {
    ToolError::Io(wrap(&format!("[browser] {error}")))
}

/// Quyết định mức rủi ro từ kết quả kiểm tra URL của lớp guard.
///
/// Đây là **điểm chuyển đổi duy nhất** Confirm↔Dangerous của domain browser.
fn risk_from_verdict(whitelisted: bool) -> Risk {
    if whitelisted {
        Risk::Confirm
    } else {
        // Ngoài whitelist: luôn Dangerous ⇒ Policy không bao giờ cho "trong phiên".
        Risk::Dangerous
    }
}

/// Mô tả ngắn lý do nâng mức rủi ro, đưa vào prompt xác nhận.
fn risk_note(whitelisted: bool, origin: &str) -> String {
    if whitelisted {
        format!("[origin {origin} nằm trong [browser].allowed_origins]")
    } else {
        format!(
            "[origin {origin} KHÔNG nằm trong [browser].allowed_origins — hành động này \
             luôn cần xác nhận, không có tuỳ chọn \"cho phép trong phiên\"]"
        )
    }
}

/// Thông điệp cho model khi lớp guard từ chối.
fn guard_error(error: &GuardError) -> ToolError {
    ToolError::InvalidArgs(format!("[browser] {error}"))
}

// ---------------------------------------------------------------------------
// browser_navigate
// ---------------------------------------------------------------------------

/// Tham số của `browser_navigate`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigateParams {
    /// URL cần mở, ví dụ `http://localhost:3000/dashboard`.
    ///
    /// URL phải qua lớp kiểm tra riêng của domain browser. Origin nằm trong
    /// `[browser].allowed_origins` thì mới được Confirm; ngoài đó luôn Dangerous.
    /// Địa chỉ trong mạng nội bộ mà không khai trong whitelist sẽ bị chặn cứng.
    pub url: String,
}

/// Mở một URL trong trình duyệt.
pub struct NavigateTool {
    session: Arc<SessionManager>,
    /// [`BrowserGuard`] đã giữ sẵn whitelist, nên `risk()` chỉ cần hỏi `guard` — không
    /// phải giữ thêm một bản whitelist ở đây (hai bản dễ lệch nhau).
    guard: Arc<BrowserGuard>,
}

impl NavigateTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>, guard: Arc<BrowserGuard>) -> Self {
        Self { session, guard }
    }

    /// Tra `url` trong `args` mà **không** cần Chrome (dùng bởi [`Tool::risk`]).
    fn peek_url(args: &Value) -> Option<String> {
        args.get("url").and_then(Value::as_str).map(str::to_string)
    }
}

#[async_trait]
impl beanagent_tools::Tool for NavigateTool {
    fn spec(&self) -> ToolSpec {
        beanagent_tools::typed_spec::<NavigateParams>("browser_navigate")
    }

    fn risk(&self, args: &Value) -> Risk {
        let verdict = Self::peek_url(args).and_then(|url| self.guard.check(&url).ok());
        risk_from_verdict(verdict.is_some_and(|v| v.whitelisted))
    }

    fn describe(&self, args: &Value) -> String {
        match Self::peek_url(args) {
            Some(url) => format!("browser_navigate: {url}"),
            None => "browser_navigate".to_string(),
        }
    }

    fn marks_untrusted(&self) -> bool {
        // Nội dung trang sau khi điều hướng là dữ liệu ngoài lõi.
        true
    }

    fn required_tags(&self) -> Vec<&str> {
        ACT_TAGS.to_vec()
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        let params: NavigateParams =
            beanagent_tools::deserialize_params(args, &self.spec().parameters)?;
        let verdict = self.guard.check(&params.url).map_err(|e| guard_error(&e))?;

        let page = self.session.page().await.map_err(launch_error)?;
        page.goto(&params.url)
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không mở được URL: {e}"))))?;
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(15),
            page.wait_for_navigation(),
        )
        .await;

        let title = page.get_title().await.ok().flatten().unwrap_or_default();
        let final_url = page.url().await.unwrap_or(None).unwrap_or_default();
        // Nhớ origin **của URL đích thật**, không phải URL người dùng gõ: redirect
        // `localhost` → `accounts.google.com` sẽ đổi mức rủi ro của lệnh click kế
        // tiếp, và đó mới là trang thật sự đang mở.
        self.session
            .set_current_origin(
                url::Url::parse(&final_url)
                    .ok()
                    .and_then(|parsed| crate::origin::parse_origin(&parsed)),
            )
            .await;
        let body = format!(
            "Đã mở {final_url}\nTiêu đề: {title}\n{}",
            risk_note(verdict.whitelisted, &verdict.origin.as_display())
        );
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(wrap(&body))
    }
}

/// Mức rủi ro cho tool hành động **không mang URL** (click, fill, press key).
///
/// Dùng origin hiện tại đã nhớ sau lần `browser_navigate` gần nhất. `None` (chưa
/// navigate, hoặc trang bị đóng) ⇒ `Dangerous` — fail-closed, không đoán bừa.
fn risk_from_current_origin(session: &SessionManager, whitelist: &OriginWhitelist) -> Risk {
    match session.current_origin_sync() {
        Some(origin) => risk_from_verdict(whitelist.verdict(&origin) == OriginVerdict::Whitelisted),
        None => Risk::Dangerous,
    }
}

// ---------------------------------------------------------------------------
// browser_click
// ---------------------------------------------------------------------------

/// Tham số của `browser_click`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClickParams {
    /// CSS selector của phần tử cần bấm, ví dụ `button#submit`.
    pub selector: String,
}

/// Bấm vào một phần tử trên trang hiện tại.
pub struct ClickTool {
    session: Arc<SessionManager>,
    whitelist: Arc<OriginWhitelist>,
}

impl ClickTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>, whitelist: Arc<OriginWhitelist>) -> Self {
        Self { session, whitelist }
    }
}

#[async_trait]
impl beanagent_tools::Tool for ClickTool {
    fn spec(&self) -> ToolSpec {
        beanagent_tools::typed_spec::<ClickParams>("browser_click")
    }

    fn risk(&self, _args: &Value) -> Risk {
        risk_from_current_origin(&self.session, &self.whitelist)
    }

    fn describe(&self, args: &Value) -> String {
        format!(
            "browser_click: {}",
            args.get("selector")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
    }

    fn marks_untrusted(&self) -> bool {
        true
    }

    fn required_tags(&self) -> Vec<&str> {
        ACT_TAGS.to_vec()
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        let params: ClickParams =
            beanagent_tools::deserialize_params(args, &self.spec().parameters)?;
        let page = self.session.page().await.map_err(launch_error)?;
        let element = page.find_element(&params.selector).await.map_err(|e| {
            ToolError::Io(wrap(&format!(
                "không tìm thấy phần tử `{}`: {e}",
                params.selector
            )))
        })?;
        element
            .click()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không bấm được: {e}"))))?;
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            page.wait_for_navigation(),
        )
        .await;

        let url_now = page.url().await.unwrap_or(None).unwrap_or_default();
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(wrap(&format!(
            "Đã bấm `{}` trên {url_now}",
            params.selector
        )))
    }
}

// ---------------------------------------------------------------------------
// browser_fill
// ---------------------------------------------------------------------------

/// Tham số của `browser_fill`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillParams {
    /// CSS selector của ô nhập, ví dụ `input[name=email]`.
    pub selector: String,
    /// Nội dung cần nhập.
    ///
    /// Đây là **dữ liệu không tin cậy**: nó có thể chứa chuỗi giống chỉ dẫn injection,
    /// nên kết quả trả về vẫn bọc `<untrusted_content>`.
    pub value: String,
}

/// Nhập văn bản vào một ô nhập trên trang hiện tại.
pub struct FillTool {
    session: Arc<SessionManager>,
    whitelist: Arc<OriginWhitelist>,
}

impl FillTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>, whitelist: Arc<OriginWhitelist>) -> Self {
        Self { session, whitelist }
    }
}

#[async_trait]
impl beanagent_tools::Tool for FillTool {
    fn spec(&self) -> ToolSpec {
        beanagent_tools::typed_spec::<FillParams>("browser_fill")
    }

    fn risk(&self, _args: &Value) -> Risk {
        risk_from_current_origin(&self.session, &self.whitelist)
    }

    fn describe(&self, args: &Value) -> String {
        let selector = args
            .get("selector")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // KHÔNG in `value`: nó có thể chứa mật khẩu người dùng gõ vào form test.
        format!("browser_fill: {selector}")
    }

    fn marks_untrusted(&self) -> bool {
        true
    }

    fn required_tags(&self) -> Vec<&str> {
        ACT_TAGS.to_vec()
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        let params: FillParams =
            beanagent_tools::deserialize_params(args, &self.spec().parameters)?;
        let page = self.session.page().await.map_err(launch_error)?;
        let element = page.find_element(&params.selector).await.map_err(|e| {
            ToolError::Io(wrap(&format!(
                "không tìm thấy ô nhập `{}`: {e}",
                params.selector
            )))
        })?;
        element
            .click()
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không focus được ô nhập: {e}"))))?;
        element
            .type_str(&params.value)
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("không nhập được: {e}"))))?;

        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(wrap(&format!(
            "Đã nhập {} ký tự vào `{}` (không in nội dung nhạy cảm)",
            params.value.chars().count(),
            params.selector
        )))
    }
}

// ---------------------------------------------------------------------------
// browser_press_key
// ---------------------------------------------------------------------------

/// Tham số của `browser_press_key`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PressKeyParams {
    /// Tên phím theo quy ước CDP, ví dụ `Enter`, `Tab`, `Escape`, `ArrowDown`.
    ///
    /// Có thể ghi kèm tổ hợp dạng `Control+Shift+R`.
    pub key: String,
}

/// Bấm một phím trên trang hiện tại.
pub struct PressKeyTool {
    session: Arc<SessionManager>,
    whitelist: Arc<OriginWhitelist>,
}

impl PressKeyTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>, whitelist: Arc<OriginWhitelist>) -> Self {
        Self { session, whitelist }
    }
}

#[async_trait]
impl beanagent_tools::Tool for PressKeyTool {
    fn spec(&self) -> ToolSpec {
        beanagent_tools::typed_spec::<PressKeyParams>("browser_press_key")
    }

    fn risk(&self, _args: &Value) -> Risk {
        risk_from_current_origin(&self.session, &self.whitelist)
    }

    fn describe(&self, args: &Value) -> String {
        format!(
            "browser_press_key: {}",
            args.get("key").and_then(Value::as_str).unwrap_or_default()
        )
    }

    fn marks_untrusted(&self) -> bool {
        true
    }

    fn required_tags(&self) -> Vec<&str> {
        ACT_TAGS.to_vec()
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        let params: PressKeyParams =
            beanagent_tools::deserialize_params(args, &self.spec().parameters)?;
        let page = self.session.page().await.map_err(launch_error)?;
        // `press_key` là method của `Element` (chiếu sự kiện bàn phím lên node), còn
        // bàn phím cấp trang thì đi qua `Input.dispatchKeyEvent`. Ở đây gửi thẳng
        // `Input` vì người dùng thường muốn phím tắt toàn cục (Esc, Tab, Enter trên
        // modal), không phải phím tắt trên một node cụ thể.
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParamsBuilder, DispatchKeyEventType,
        };
        for event_type in [DispatchKeyEventType::KeyDown, DispatchKeyEventType::KeyUp] {
            // Builder của crate CDP có field private nên phải đi qua setter, và
            // `build()` trả `Result` (thiếu trường bắt buộc sẽ lỗi).
            let params = DispatchKeyEventParamsBuilder::default()
                .r#type(event_type)
                .key(params.key.clone())
                .build()
                .map_err(|e| ToolError::Io(wrap(&format!("tham số phím không hợp lệ: {e}"))))?;
            page.execute(params)
                .await
                .map_err(|e| ToolError::Io(wrap(&format!("không bấm được phím: {e}"))))?;
        }
        let _ = tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            page.wait_for_navigation(),
        )
        .await;

        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(wrap(&format!("Đã bấm phím `{}`", params.key)))
    }
}

// ---------------------------------------------------------------------------
// browser_evaluate_script
// ---------------------------------------------------------------------------

/// Tham số của `browser_evaluate_script`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluateParams {
    /// Biểu thức JavaScript chạy trong ngữ cảnh trang hiện tại.
    ///
    /// Biểu thức được bọc trong `(() => { return (…) })()` nên phải có lệnh `return`.
    /// Dùng `awaitPromise: true` nên có thể `return await fetch(...)`.
    pub expression: String,
}

/// Chạy JavaScript trong trang hiện tại.
///
/// # Vì sao LUÔN `Dangerous`, kể cả origin trong whitelist
///
/// Chạy JS tuỳ ý **tương đương thực thi mã**: nó đọc được cookie phiên, localStorage
/// và gọi được API nội bộ bằng chính quyền của người dùng đang đăng nhập. Không có
/// khái niệm "origin này đáng tin tới mức cho chạy mã" — whitelist chỉ nói "trang này
/// là môi trường test của bạn", không nói "an toàn để thực thi".
///
/// Vì vậy `Policy` sẽ **không bao giờ** hiện tuỳ chọn "cho phép trong phiên" cho tool
/// này, và một lượt hàng trăm lệnh sẽ phải bấm Duyệt từng lần — đó là cái giá đúng.
pub struct EvaluateScriptTool {
    session: Arc<SessionManager>,
}

impl EvaluateScriptTool {
    /// Dựng tool.
    #[must_use]
    pub fn new(session: Arc<SessionManager>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl beanagent_tools::Tool for EvaluateScriptTool {
    fn spec(&self) -> ToolSpec {
        beanagent_tools::typed_spec::<EvaluateParams>("browser_evaluate_script")
    }

    fn risk(&self, _args: &Value) -> Risk {
        // Cứng `Dangerous`, không phụ thuộc tham số lẫn origin.
        Risk::Dangerous
    }

    fn describe(&self, args: &Value) -> String {
        // In biểu thức ở dạng rút gọn cho UI, nhưng đủ để người duyệt hiểu đang chạy gì.
        let expression = args
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match beanagent_tools::truncate_chars(expression, 120) {
            Some((short, _)) => format!("browser_evaluate_script: {short}…"),
            None => format!("browser_evaluate_script: {expression}"),
        }
    }

    fn marks_untrusted(&self) -> bool {
        true
    }

    fn required_tags(&self) -> Vec<&str> {
        ACT_TAGS.to_vec()
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        let params: EvaluateParams =
            beanagent_tools::deserialize_params(args, &self.spec().parameters)?;
        let page = self.session.page().await.map_err(launch_error)?;
        let result = page
            .evaluate(params.expression.clone())
            .await
            .map_err(|e| ToolError::Io(wrap(&format!("lỗi JavaScript: {e}"))))?;

        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // `into_value` của EvaluationResult có thể chứa `exception_details` — vẫn bọc
        // untrusted vì do trang kiểm soát.
        // `into_value` là `Result` (có thể lỗi deserialize) — không `unwrap`.
        let rendered = result
            .into_value()
            .map_err(|e| ToolError::Io(wrap(&format!("không đọc được kết quả JS: {e}"))))
            .and_then(|value: serde_json::Value| {
                serde_json::to_string_pretty(&value)
                    .map_err(|e| ToolError::Io(wrap(&format!("không render được kết quả JS: {e}"))))
            })?;
        Ok(wrap(&rendered))
    }
}
