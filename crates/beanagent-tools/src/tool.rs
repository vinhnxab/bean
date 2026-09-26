//! Trait [`Tool`] — hợp đồng mà mọi công cụ phải tuân theo (agents.md mục 7.1).

use async_trait::async_trait;
use beanagent_types::{Risk, ToolSpec};

use crate::ctx::ToolCtx;
use crate::error::ToolError;

/// Một công cụ mà model gọi được.
///
/// Bất biến:
/// * `spec()` ổn định trong suốt tiến trình (schema gửi model không đổi giữa chừng).
/// * `call` **không được panic**; mọi lỗi trả về [`ToolError`] và agent loop sẽ biến thành
///   tool result `is_error` (mục 6 — lỗi tool không làm hỏng vòng lặp).
/// * `args` là JSON **không tin cậy** do model sinh — phải validate (thường qua
///   [`crate::typed::deserialize_params`]) trước khi dùng.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Mô tả tool (tên + description + JSON Schema tham số).
    fn spec(&self) -> ToolSpec;

    /// Mức rủi ro; **có thể phụ thuộc tham số** (mục 7.1).
    fn risk(&self, args: &serde_json::Value) -> Risk;

    /// Tóm tắt một dòng về lời gọi này cho UI/log (`ToolStart.summary`, mục 10).
    /// Mặc định: tên tool.
    fn describe(&self, _args: &serde_json::Value) -> String {
        self.spec().name
    }

    /// Tool này có trả **nội dung từ nguồn ngoài lõi** (web, file, output lệnh, email,
    /// MCP) không — mục 15.4?
    ///
    /// Mặc định `false`. Tool trả `true` **phải** thỏa hai điều kiện:
    ///
    /// 1. output trả về được bọc trong `<untrusted_content>` bằng
    ///    [`crate::untrusted::wrap`] hoặc [`crate::untrusted::wrap_bounded`]
    ///    (tự bọc trong `call`, hoặc dùng builder `TypedTool::untrusted`);
    /// 2. tool tự bật `ctx.untrusted_seen` (theo đúng cách `web_fetch` đang làm).
    ///
    /// Agent loop còn dùng cờ này như **lưới an toàn thứ hai**: kể cả khi tool quên
    /// bật cờ, `run_turn` vẫn bật được từ khai báo ở đây. Nhờ vậy tool mới quên bọc sẽ
    /// bị test hồi quy phát hiện, thay vì hỏng âm thầm.
    fn marks_untrusted(&self) -> bool {
        false
    }

    /// Thực thi tool.
    ///
    /// # Errors
    /// Bất kỳ lỗi nào (tham số sai, file thiếu, timeout…) — được trả về thay vì panic.
    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError>;
}
