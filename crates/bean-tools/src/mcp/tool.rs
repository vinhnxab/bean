//! `McpTool`: wrapper `Tool` cho mot tool MCP tu xa.

use std::{borrow::Cow, sync::Arc};

use async_trait::async_trait;
use bean_types::{Risk, ToolSpec};
use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};

use super::config::McpCallFailure;
use super::connection::McpConnection;
use crate::{
    ToolCtx, ToolError,
    tool::{Tool, ToolAccess},
    untrusted::wrap,
};

/// Adapter từ một MCP tool definition sang `bean_tools::Tool`.
pub(crate) struct McpTool {
    connection: Arc<McpConnection>,
    remote_name: String,
    spec: ToolSpec,
    trusted: bool,
    /// Tag RBAC kế thừa từ `[[mcp_servers]].tool_tags` (M22).
    ///
    /// Dùng `Vec<String>` (không phải `&'static str`) vì tag đến từ file cấu hình chạy
    /// được, không phải literal trong mã.
    required_tags: Arc<Vec<String>>,
}

pub(crate) const fn risk_for_trust(trusted: bool) -> Risk {
    if trusted { Risk::Safe } else { Risk::Confirm }
}

/// Kết quả tool có khối text **đọc được** không.
///
/// `false` nghĩa là server trả về im lặng: mọi khối text đều rỗng hoặc chỉ toàn khoảng
/// trắng. Đây là tín hiệu duy nhất phân biệt được "tool lỗi" với "tool lỗi nhưng server
/// nuốt mất lý do" — xem [`McpTool::silent_remote_error`].
///
/// Ảnh/âm thanh/resource vẫn tính là **có** nội dung (model nhìn được, mục 26); chỉ
/// text rỗng mới bị coi là im lặng.
pub(crate) fn has_readable_text(content: &[ContentBlock]) -> bool {
    content.iter().any(|block| match block {
        ContentBlock::Text(text) => !text.text.trim().is_empty(),
        _ => true,
    })
}

/// Payload thay thế khi MCP server báo lỗi mà không kèm message.
///
/// Giữ nguyên `raw` để không đánh mất thông tin, và nói rõ cho model biết phải làm gì:
/// đây là lỗi *cục bộ của server*, không phải hành động bị từ chối — nên cách sửa đúng là
/// kiểm tra tham số, đặc biệt là đường dẫn phải **tuyệt đối** cho tool ngoài core.
pub(crate) fn silent_error_payload(server: &str, raw: &CallToolResult) -> serde_json::Value {
    serde_json::json!({
        "isError": true,
        "error": format!(
            "MCP server `{server}` báo tool thất bại nhưng KHÔNG kèm message lỗi. \
             Server bỏ sót chi tiết ở chế độ rút gọn (ví dụ chrome-devtools-mcp với `--slim`). \
             Hãy kiểm tra lại tham số; với `url`/`file` phải dùng đường dẫn TUYỆT ĐỐI trên máy chủ \
             (xem mục `# Environment` của system prompt), đừng suy ra từ đường dẫn tương đối."
        ),
        "raw": raw,
    })
}

impl McpTool {
    pub(crate) fn new(
        connection: Arc<McpConnection>,
        remote_name: String,
        spec: ToolSpec,
        trusted: bool,
        required_tags: Arc<Vec<String>>,
    ) -> Self {
        Self {
            connection,
            remote_name,
            spec,
            trusted,
            required_tags,
        }
    }

    fn mark_untrusted(ctx: &ToolCtx) {
        ctx.untrusted_seen
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn remote_error(ctx: &ToolCtx, payload: serde_json::Value) -> ToolError {
        Self::mark_untrusted(ctx);
        let encoded = serde_json::to_string(&payload)
            .unwrap_or_else(|_| "{\"error\":\"không serialize được kết quả MCP\"}".to_string());
        ToolError::Mcp(wrap(&encoded))
    }

    /// `isError: true` mà **không kèm message lỗi** — server nuốt mất lý do.
    ///
    /// Xảy ra thật với `chrome-devtools-mcp` ở chế độ `--slim` (đã bật trong `bean.toml`):
    /// `SlimMcpResponse.handle()` chỉ serialize `responseLines` — rỗng khi handler ném lỗi —
    /// và **không** kèm message, trong khi `McpResponse.handle()` ở chế độ thường có
    /// `errorMessage: this.#error?.message`. Đã kiểm tra cả bản 1.10.1: y hệt, chưa sửa.
    ///
    /// Trả về đúng thông báo lỗi cho model thay vì chuỗi rỗng — nếu không, model và UI chỉ
    /// thấy `{"content":[{"type":"text","text":""}],"isError":true}` và không có cách nào
    /// đoán nguyên nhân.
    fn silent_remote_error(ctx: &ToolCtx, server: &str, raw: &CallToolResult) -> ToolError {
        Self::mark_untrusted(ctx);
        let encoded =
            serde_json::to_string(&silent_error_payload(server, raw)).unwrap_or_else(|_| {
                "{\"isError\":true,\"error\":\"MCP server báo lỗi mà không kèm message\"}"
                    .to_string()
            });
        ToolError::Mcp(wrap(&encoded))
    }

    fn call_failure(&self, failure: McpCallFailure) -> ToolError {
        let detail = match failure {
            McpCallFailure::Timeout => format!(
                "MCP tool `{}` treo sau {} ms",
                self.spec.name,
                self.connection.call_timeout().as_millis()
            ),
            McpCallFailure::Transport(message) => {
                format!("MCP tool `{}` lỗi transport: {message}", self.spec.name)
            }
        };
        ToolError::Mcp(detail)
    }
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn risk(&self, _args: &serde_json::Value) -> Risk {
        risk_for_trust(self.trusted)
    }

    fn describe(&self, _args: &serde_json::Value) -> String {
        format!(
            "MCP server `{}`, tool `{}`",
            self.connection.server_name(),
            self.remote_name
        )
    }

    /// MCP luôn là nguồn ngoài lõi (mục 15.4/16): kết quả **và cả lỗi** đều đã bọc
    /// `<untrusted_content>` trong `call`, nên khai báo `true` để agent loop bật cờ
    /// ngay cả khi `call` trả `Err` trước khi tới chỗ bọc.
    fn marks_untrusted(&self) -> bool {
        true
    }

    /// (M22) Tag RBAC kế thừa từ `[[mcp_servers]].tool_tags`.
    ///
    /// Rỗng ⇒ mọi role đã cấp quyền đều thấy (giữ hành vi cũ cho server không gắn tag).
    ///
    /// Khác với tool viết tay, tag ở đây đến từ **cấu hình chạy được** nên là `String`
    /// của runtime, không phải literal `'static`. Vì vậy phải `Cow::Owned` — đây là
    /// lý do [`ToolAccess`] dùng `Cow` thay vì `&'static [&'static str]`.
    fn access(&self) -> ToolAccess<'_> {
        ToolAccess {
            required_tags: Cow::Owned(self.required_tags.iter().map(String::as_str).collect()),
            also_visible_to: Cow::Borrowed(&[]),
        }
    }

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError> {
        let serde_json::Value::Object(arguments) = args else {
            return Err(ToolError::InvalidArgs(format!(
                "arguments phải là JSON object theo schema: {}",
                self.spec.parameters
            )));
        };
        let mut retried = false;
        let response = loop {
            match self
                .connection
                .call_tool(&self.remote_name, arguments.clone())
                .await
            {
                Ok(response) => break response,
                Err(failure) if !retried => {
                    retried = true;
                    tracing::warn!(
                        server = %self.connection.server_name(),
                        tool = %self.spec.name,
                        ?failure,
                        "MCP transport rớt; thực hiện reconnect một lần"
                    );
                    if let Err(error) = self.connection.reconnect().await {
                        return Err(ToolError::Mcp(format!(
                            "MCP server `{}` reconnect thất bại: {error}",
                            self.connection.server_name()
                        )));
                    }
                }
                Err(failure) => return Err(self.call_failure(failure)),
            }
        };

        match response {
            CallToolResponse::Complete(result) => {
                let is_error = result.is_error.unwrap_or(false);
                // Server báo lỗi nhưng không kèm lý do: phải nói rõ thay vì trả chuỗi
                // rỗng (xem `silent_remote_error`).
                if is_error && !has_readable_text(&result.content) {
                    return Err(Self::silent_remote_error(
                        ctx,
                        self.connection.server_name(),
                        &result,
                    ));
                }
                let encoded = match serde_json::to_string(&result) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        return Err(Self::remote_error(
                            ctx,
                            serde_json::json!({ "error": error.to_string() }),
                        ));
                    }
                };
                Self::mark_untrusted(ctx);
                let wrapped = wrap(&encoded);
                if is_error {
                    Err(ToolError::Mcp(wrapped))
                } else {
                    Ok(wrapped)
                }
            }
            CallToolResponse::InputRequired(_) => Err(Self::remote_error(
                ctx,
                serde_json::json!({
                    "error": "MCP server yêu cầu input tương tác; Bean chưa hỗ trợ MRTR input_required"
                }),
            )),
            CallToolResponse::Task(_) => Err(Self::remote_error(
                ctx,
                serde_json::json!({
                    "error": "MCP server trả task handle; Bean chưa hỗ trợ task polling"
                }),
            )),
            _ => Err(Self::remote_error(
                ctx,
                serde_json::json!({ "error": "MCP server trả loại kết quả không hỗ trợ" }),
            )),
        }
    }
}
