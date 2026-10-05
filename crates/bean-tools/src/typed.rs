//! [`TypedTool<P>`] — bộ chuyển đổi từ một handler kiểu hoá sang [`Tool`](crate::Tool)
//! (agents.md mục 7.1).
//!
//! Schema sinh bằng `schemars` từ struct tham số `P`:
//! * doc comment của struct ⇒ `description` gửi cho model — hãy viết rõ *khi nào dùng*;
//! * doc comment của từng field ⇒ `description` của tham số;
//! * `#[serde(deny_unknown_fields)]` trên `P` ⇒ `additionalProperties: false` trong schema
//!   **và** serde sẽ trả lỗi khi model gửi tham số thừa.
//!
//! Lưu ý khi viết handler: async block phải **không mượn `ctx`** (extract dữ liệu cần thiết
//! trước), để future vẫn là kiểu `'static` khớp bound của `TypedTool`.

#![allow(clippy::type_complexity)]

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::{Tool, ToolAccess};
use bean_types::{Risk, ToolSpec};
use std::borrow::Cow;

/// Sinh [`ToolSpec`] từ struct tham số `P: JsonSchema` (D6.10: schema giữ thô, provider
/// chịu trách nhiệm chuẩn hoá `$defs`/`$ref` trước khi gửi API).
///
/// `description` lấy từ doc comment của struct `P` (schemars tự đưa vào trường
/// `description` của schema); nếu thiếu thì rỗng — khi đó model chỉ đọc được tên tool.
#[must_use]
pub fn typed_spec<P: JsonSchema>(name: &str) -> ToolSpec {
    let schema = schemars::schema_for!(P);
    // Serialize của `Schema` (bọc `serde_json::Value`) không thể thất bại trong thực tế;
    // vẫn dùng fallback để không vi phạm quy tắc "không unwrap" (mục 0.8).
    let value = serde_json::to_value(&schema).unwrap_or_else(|_| serde_json::json!({}));
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    ToolSpec::new(name, description, value)
}

/// Deserialize tham số từ JSON của model — **đầu vào không tin cậy** (mục 22.11).
///
/// Lỗi trả về kèm danh sách tham số hợp lệ (từ schema) để model tự sửa.
///
/// # Errors
/// [`ToolError::InvalidArgs`] khi JSON sai schema (thiếu/thừa/sai kiểu).
pub fn deserialize_params<P: DeserializeOwned>(
    args: Value,
    schema: &Value,
) -> Result<P, ToolError> {
    serde_json::from_value(args).map_err(|err| {
        let valid = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| props.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        ToolError::InvalidArgs(format!("{err}. Tham số hợp lệ: {valid}"))
    })
}
/// Tool kiểu hoá: handler nhận tham số đã deserialize thay vì JSON thô.
///
/// ```ignore
/// let tool = TypedTool::<GreetParams, _, _>::new("greet", Risk::Safe, |_ctx, p| async move {
///     Ok(format!("xin chào {}", p.name))
/// });
/// ```
pub struct TypedTool<P, F, Fut> {
    name: String,
    spec: ToolSpec,
    default_risk: Risk,
    risk_fn: Option<Arc<dyn Fn(&Value) -> Risk + Send + Sync>>,
    marks_untrusted: bool,
    required_tags: Vec<&'static str>,
    also_visible_to: Vec<&'static str>,
    handler: F,
    _phantom: PhantomData<fn(P) -> Fut>,
}

impl<P, F, Fut> TypedTool<P, F, Fut>
where
    P: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    F: for<'a> Fn(&'a ToolCtx, P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String, ToolError>> + Send + 'static,
{
    /// Tạo tool với mức rủi ro cố định.
    #[must_use]
    pub fn new(name: &str, risk: Risk, handler: F) -> Self {
        Self::build(name, risk, None, handler)
    }

    /// Tạo tool với mức rủi ro **phụ thuộc tham số** (mục 7.1).
    #[must_use]
    pub fn with_risk_fn(
        name: &str,
        risk_fn: impl Fn(&Value) -> Risk + Send + Sync + 'static,
        handler: F,
    ) -> Self {
        Self::build(name, Risk::Safe, Some(Arc::new(risk_fn)), handler)
    }

    /// Khai báo tool trả nội dung từ **nguồn ngoài lõi** (mục 15.4).
    ///
    /// `TypedTool::call` sẽ tự động bọc output bằng
    /// [`crate::untrusted::wrap_bounded`] và bật `ctx.untrusted_seen` — tác giả tool
    /// không thể quên một trong hai việc, và [`Tool::marks_untrusted`] trả `true`.
    ///
    /// ```ignore
    /// let tool = TypedTool::new("read_file", Risk::Safe, handler).untrusted();
    /// ```
    #[must_use]
    pub const fn untrusted(mut self) -> Self {
        self.marks_untrusted = true;
        self
    }

    /// Chỉ **khai báo** là nguồn ngoài lõi mà không để `TypedTool` tự bọc.
    ///
    /// Dùng cho tool đã tự bọc thẻ bằng tay ở trong `handler` (kể cả nhánh lỗi) —
    /// hiện là `web_fetch`/`web_search` với `wrap_untrusted_limited`. Agent loop vẫn
    /// bật `untrusted_seen` từ khai báo này; khác [`Self::untrusted`] ở chỗ không bọc
    /// lần hai.
    #[must_use]
    pub const fn declares_untrusted(mut self) -> Self {
        self.marks_untrusted = true;
        self
    }

    /// Khai báo tag RBAC mà role phải giữ **ít nhất một** để thấy tool này (M21.4).
    ///
    /// ```ignore
    /// // four-eyes: chỉ developer (dev-write) và security-scan (infra-scan) chạy được
    /// // lệnh shell; qa bị chặn dù có thể đọc code.
    /// let tool = TypedTool::new("run_shell", Risk::Confirm, handler)
    ///     .requires_tags(["dev-write", "infra-scan"]);
    /// ```
    #[must_use]
    pub fn requires_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<&'static str>,
    {
        self.required_tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Ghi đè `description` gửi cho model (M27).
    ///
    /// `TypedTool::build` lấy description từ doc comment của struct tham số. Có tool cần
    /// mô tả **động** theo cấu hình lúc chạy — ví dụ `qa_test` liệt kê đúng danh sách
    /// `[[qa.suites]]` đang khai báo, để model không phải đoán mò tên suite (D14.1).
    ///
    /// Chỉ ghi đè phần mô tả; schema tham số giữ nguyên do `schemars` sinh ra.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.spec.description = description.into();
        self
    }

    /// Mở thêm tag **bổ sung** để tool untagged vẫn hiện với một role cụ thể (M24).
    #[must_use]
    pub fn also_visible_to<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<&'static str>,
    {
        self.also_visible_to = tags.into_iter().map(Into::into).collect();
        self
    }

    fn build(
        name: &str,
        default_risk: Risk,
        risk_fn: Option<Arc<dyn Fn(&Value) -> Risk + Send + Sync>>,
        handler: F,
    ) -> Self {
        Self {
            name: name.to_string(),
            spec: typed_spec::<P>(name),
            default_risk,
            risk_fn,
            marks_untrusted: false,
            required_tags: Vec::new(),
            also_visible_to: Vec::new(),
            handler,
            _phantom: PhantomData,
        }
    }

    /// Tóm tắt mặc định cho `describe`: ghép `khoá=giá trị` (giá trị cắt 40 ký tự).
    fn summary_from_args(&self, args: &Value) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(map) = args.as_object() {
            for (key, value) in map {
                let rendered = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let (cut, was_cut) = crate::text::truncate_chars(&rendered, 40)
                    .map_or((rendered.as_str(), false), |(kept, _)| (kept, true));
                let suffix = if was_cut { "…" } else { "" };
                parts.push(format!("{key}={cut}{suffix}"));
            }
        }
        let joined = parts.join("; ");
        if let Some((kept, _)) = crate::text::truncate_chars(&joined, 120) {
            format!("{kept}…")
        } else {
            joined
        }
    }
}

#[async_trait]
impl<P, F, Fut> Tool for TypedTool<P, F, Fut>
where
    P: DeserializeOwned + JsonSchema + Send + Sync + 'static,
    F: for<'a> Fn(&'a ToolCtx, P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String, ToolError>> + Send + 'static,
{
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn risk(&self, args: &Value) -> Risk {
        match &self.risk_fn {
            Some(f) => f(args),
            None => self.default_risk,
        }
    }

    fn describe(&self, args: &Value) -> String {
        let summary = self.summary_from_args(args);
        if summary.is_empty() {
            return self.name.clone();
        }
        format!("{}: {summary}", self.name)
    }

    fn marks_untrusted(&self) -> bool {
        self.marks_untrusted
    }

    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Borrowed(&self.required_tags),
            also_visible_to: Cow::Borrowed(&self.also_visible_to),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<String, ToolError> {
        let params: P = deserialize_params(args, &self.spec.parameters)?;
        let output = (self.handler)(ctx, params).await?;
        if !self.marks_untrusted {
            return Ok(output);
        }
        // (mục 15.4) Nội dung từ nguồn ngoài lõi: bật cờ cho **cả lượt** trước, rồi bọc
        // thẻ. Cờ bật ở đây nên kể cả khi `handler` nội bộ có bỏ sót, agent loop vẫn
        // biết lượt này đã đọc dữ liệu không tin cậy.
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(crate::untrusted::wrap_bounded(
            &output,
            crate::untrusted::MAX_WRAPPED_OUTPUT_CHARS,
        ))
    }
}
