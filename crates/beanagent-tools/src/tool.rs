//! Trait [`Tool`] — hợp đồng mà mọi công cụ phải tuân theo (agents.md mục 7.1).

use async_trait::async_trait;
use beanagent_types::{Risk, ToolSpec};

use crate::ctx::ToolCtx;
use crate::error::ToolError;

/// Nội dung một tool trả về: **văn bản** (mặc định) hoặc **ảnh** (M26).
///
/// # Vì sao có enum này mà không đổi chữ ký `Tool::call`
///
/// `Tool::call` trả `Result<String, ToolError>`. Đổi nó thành `Result<ToolOutput, _>`
/// sẽ chạm vào **mọi** impl `Tool` (25+ tool built-in, `TypedTool`, wrapper MCP),
/// mọi adapter kênh và hàng chục test — trong khi nhu cầu thật chỉ có **một** tool
/// (`browser_screenshot`) cần trả ảnh.
///
/// Vì vậy [`Tool::call_rich`] được thêm với **default implementation** gọi lại
/// `call`; chỉ tool trả ảnh mới override. Phạm vi thay đổi thu hẹp còn **một impl**.
///
/// # Ràng buộc bất biến
///
/// * `Text` đi qua đúng đường cũ: `untrusted::wrap_bounded` + `agent::truncate_output`.
/// * `Image` **không** bị cắt theo ký tự (cắt byte ảnh là vô nghĩa) và **không** tính
///   vào ngân sách token bằng công thức `chars/4` — xem `context::image_output_tokens`.
/// * `media_type` phải nằm trong tập được phép; `ImageBlock::new` chặn ở đây chứ không
///   để chuỗi lạ tới tận provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutput {
    /// Văn bản thuần (đa số tool).
    Text(String),
    /// Một khối ảnh kèm caption ngắn cho model.
    Image {
        /// Caption một dòng (URL, kích thước, kích thước file) đi kèm ảnh.
        caption: String,
        /// Khối ảnh đã mã hoá base64.
        image: beanagent_types::ImageBlock,
    },
}

impl ToolOutput {
    /// Văn bản thuần — tiện cho code chỉ cần text.
    #[must_use]
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Image { caption, .. } => caption,
        }
    }

    /// Có ảnh hay không (agent loop dùng để chọn cách ghi audit).
    #[must_use]
    pub const fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }
}

impl From<String> for ToolOutput {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

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

    /// Tag RBAC **bổ sung**: cho phép một role cụ thể thấy tool này mà không cần mở
    /// `required_tags` (M24).
    ///
    /// # Vì sao cần, khi đã có `required_tags`?
    ///
    /// `required_tags` là **cổng chặn**: gắn tag vào `web_fetch` sẽ *giấu nó khỏi mọi role
    /// khác* — hồi quy cho các cài đặt đang chạy. Nhưng M24 đòi role `marketing` được thấy
    /// `web_fetch` mà **không** thấy `write_file`/`run_shell`. Hai yêu cầu đó không thể
    /// cùng đúng với một cơ chế.
    ///
    /// Vì vậy tách domain làm ở phía **role** (`RolePermissions::allowed_tool_tags` — danh
    /// sách trắng), còn method này chỉ *mở thêm* một lối cho tool untagged vốn bị ẩn.
    ///
    /// Mặc định `&[]` ở **mọi** tool ⇒ thêm cơ chế này không đổi hành vi cấu hình cũ nào.
    fn also_visible_to(&self) -> Vec<&str> {
        Vec::new()
    }

    /// Thực thi tool.
    ///
    /// # Errors
    /// Bất kỳ lỗi nào (tham số sai, file thiếu, timeout…) — được trả về thay vì panic.
    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError>;

    /// Thực thi tool, cho phép trả **ảnh** (M26).
    ///
    /// Default implementation gọi lại [`Tool::call`] rồi bọc thành
    /// [`ToolOutput::Text`] — nên **tool cũ không cần đổi một dòng nào** và hành vi
    /// của chúng giữ nguyên tuyệt đối. Chỉ tool trả ảnh (`browser_screenshot`) override
    /// hàm này.
    ///
    /// Agent loop gọi `call_rich` (không phải `call`) nên không cần biết tool nào trả
    /// ảnh: quyết định nằm ở dữ liệu trả về, không nằm ở tên tool — đổi tên tool sau
    /// này không làm hỏng đường ảnh.
    ///
    /// # Errors
    /// Như [`Tool::call`].
    async fn call_rich(
        &self,
        ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        self.call(ctx, args).await.map(ToolOutput::Text)
    }
}
