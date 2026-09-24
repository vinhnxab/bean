//! Giao tiếp giữa vòng lặp agent và kênh nhập/xuất (CLI, web, Telegram).
//!
//! `RunIo` là trait do **Router** sở hữu (D1.2), agent loop gọi nó để đưa tin nhắn
//! ra, phát sự kiện tiến trình và yêu cầu xác nhận. Mỗi run có `RouterIo` riêng nên
//! subscriber rớt không làm thay đổi vòng đời run.

use std::time::Duration;

use beanagent_types::Risk;
use tokio_util::sync::CancellationToken;

/// Quyết định của người dùng khi agent yêu cầu xác nhận.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Cho phép thực hiện hành động lần này.
    Allow,
    /// Cho phép thực hiện trong toàn phiên (chỉ dùng được với tool `Confirm`, không phải
    /// `Dangerous` — agents.md mục 7.2).
    AllowInSession,
    /// Từ chối hành động.
    Deny,
}

/// Giao diện nhập/xuất của vòng lặp agent.
#[async_trait::async_trait]
pub trait RunIo: Send + Sync + 'static {
    /// Hiển thị văn bản cho người dùng (tin nhắn streaming, final answer...).
    fn on_text(&self, text: &str);

    /// Thông báo bắt đầu chạy tool.
    fn on_tool_start(&self, id: &str, tool: &str, risk: Risk, summary: &str, args: &str);

    /// Thông báo kết quả tool.
    fn on_tool_end(&self, id: &str, tool: &str, ok: bool, output: &str);

    /// Yêu cầu xác nhận trước hành động. `timeout` là thời gian chờ (thường 300s).
    /// Trả về `None` nếu hết thời gian chờ, `Some(Decision)` nếu người dùng phản hồi.
    async fn confirm(
        &self,
        id: &str,
        tool: &str,
        risk: Risk,
        prompt: &str,
        allow_in_session: bool,
        timeout: Duration,
    ) -> Option<Decision>;

    /// Actor đã trả lời confirm gần nhất; adapter thường trả `None`.
    fn decision_actor(&self) -> Option<String> {
        None
    }

    /// Nhận `CancellationToken` để kiểm tra huỷ giữa chừng (adapter có thể ignore nếu
    /// không hỗ trợ huỷ — ví dụ web không huỷ khi đóng tab).
    fn cancel_token(&self) -> &CancellationToken;
}
