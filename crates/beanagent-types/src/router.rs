//! Kiểu dữ liệu chung cho Router, Channel và WebSocket (agents.md mục 10–11).
//!
//! `RunEvent` là nguồn sự thật duy nhất cho vòng đời một run. Mọi variant đều mang
//! `session_id` và `run_id`; `message_id` chỉ xuất hiện khi DB đã cấp được id.

use serde::{Deserialize, Serialize};

use crate::{ConfirmId, Risk, RunId, SessionId};

/// Kết quả cuối cùng của một yêu cầu xác nhận.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmOutcome {
    /// Người dùng cho phép hành động.
    Allowed,
    /// Người dùng từ chối hành động.
    Denied,
    /// Không có phản hồi trước thời hạn; tương đương từ chối.
    Expired,
}

/// Sự kiện phát từ Router qua `tokio::sync::broadcast`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    /// Run đang chờ sau run trước trong cùng session.
    Queued {
        /// Session sở hữu hàng đợi.
        session_id: SessionId,
        /// Run đang chờ.
        run_id: RunId,
        /// Vị trí trong hàng đợi, bắt đầu từ 1.
        position: u32,
    },
    /// Văn bản model phát trước khi gọi tool.
    Text {
        /// Session của run.
        session_id: SessionId,
        /// Run phát sự kiện.
        run_id: RunId,
        /// Nội dung text.
        text: String,
    },
    /// Tool call bắt đầu.
    ToolStart {
        /// Session của run.
        session_id: SessionId,
        /// Run phát sự kiện.
        run_id: RunId,
        /// Id call từ provider.
        id: String,
        /// Tên tool.
        tool: String,
        /// Tóm tắt hành động.
        summary: String,
        /// Tham số đã cắt preview.
        args_preview: String,
        /// Mức rủi ro thực tế của tool.
        risk: Risk,
    },
    /// Tool call kết thúc.
    ToolEnd {
        /// Session của run.
        session_id: SessionId,
        /// Run phát sự kiện.
        run_id: RunId,
        /// Id call để ghép với `ToolStart`.
        id: String,
        /// Tên tool.
        tool: String,
        /// Tool có thành công không.
        ok: bool,
        /// Output đã cắt preview.
        output_preview: String,
    },
    /// Router đang chờ người dùng xác nhận hành động.
    ConfirmRequest {
        /// Session của run.
        session_id: SessionId,
        /// Run đang chờ.
        run_id: RunId,
        /// Id ngẫu nhiên dùng để trả lời.
        confirm_id: ConfirmId,
        /// Toàn văn hành động cần người dùng kiểm tra.
        prompt: String,
        /// Mức rủi ro do tool khai báo.
        risk: Risk,
        /// Có được chọn “cho phép trong phiên” không.
        allow_session_option: bool,
        /// Số giây được chờ trước khi thành DENY.
        timeout_seconds: u32,
    },
    /// Một confirm đã được phân giải đúng một lần.
    ConfirmResolved {
        /// Session của run.
        session_id: SessionId,
        /// Run sở hữu confirm.
        run_id: RunId,
        /// Confirm đã resolve.
        confirm_id: ConfirmId,
        /// Kết quả cuối cùng.
        outcome: ConfirmOutcome,
    },
    /// Run kết thúc với câu trả lời cuối.
    Final {
        /// Session của run.
        session_id: SessionId,
        /// Run kết thúc.
        run_id: RunId,
        /// Nội dung trả lời.
        text: String,
        /// Message assistant cuối trong DB nếu đã lấy được id.
        message_id: Option<i64>,
    },
    /// Run kết thúc do lỗi.
    Error {
        /// Session của run.
        session_id: SessionId,
        /// Run bị lỗi.
        run_id: RunId,
        /// Mã lỗi ổn định cho adapter.
        code: String,
        /// Thông báo đã không chứa secret.
        message: String,
    },
}

/// Tin nhắn chủ động mà Router yêu cầu một `Channel` gửi đi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outbound {
    /// Session mà tin nhắn thuộc về.
    pub session_id: SessionId,
    /// Message đã lưu trong DB.
    pub message_id: i64,
    /// Nội dung cần gửi.
    pub text: String,
    /// Phân loại tin.
    pub kind: OutboundKind,
}

/// Loại tin chủ động.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundKind {
    /// Tin từ scheduler hoặc learning loop.
    Notification,
    /// Câu trả lời đã lưu của một run.
    AgentReply,
}
