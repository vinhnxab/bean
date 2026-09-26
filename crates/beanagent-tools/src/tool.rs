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

    /// Tag RBAC mà role phải giữ **ít nhất một** để thấy/gọi tool này (M21.4).
    ///
    /// Mặc định `&[]` ⇒ *"không cần thẻ đặc biệt"*: mọi role **đã được cấp quyền** đều thấy
    /// (giữ hành vi cũ cho tool chat thường — M21.4 ghi rõ điều này).
    ///
    /// **Không** đọc ngữ nghĩa "ai cũng gọi được" theo nghĩa đen: role `no-access` (user
    /// không có trong `agent.user_roles`) vẫn **không thấy tool nào**, kể cả untagged — đó là
    /// bất biến an toàn mặc định của `Plan.md` mục 2, xem D10.2.
    ///
    /// Ngữ nghĩa giữa nhiều tag là **OR**: role giữ một tag là đủ. Nhờ vậy `run_shell` có thể
    /// mang cả `dev-write` lẫn `infra-scan` mà vẫn chặn được `qa` (four-eyes).
    ///
    /// Trả `Vec<&str>` (sở hữu) thay vì `&[&str]` vì tag có thể đến từ **cấu hình chạy
    /// được** (ví dụ `[[mcp_servers]].tool_tags`) chứ không chỉ literal trong mã.
    ///
    /// Việc kiểm tra thực hiện ở **một** chỗ duy nhất: Router resolve
    /// [`RolePermissions`] rồi lọc danh sách tool **trước** khi dựng request tới LLM
    /// (M21.5) — không có logic RBAC nào nằm trong tool/role.
    fn required_tags(&self) -> Vec<&str> {
        Vec::new()
    }

    /// Thực thi tool.
    ///
    /// # Errors
    /// Bất kỳ lỗi nào (tham số sai, file thiếu, timeout…) — được trả về thay vì panic.
    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError>;
}
