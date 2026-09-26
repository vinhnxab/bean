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
    /// Văn bản model phát trước khi gọi tool (snapshot non-stream cũ).
    Text {
        /// Session của run.
        session_id: SessionId,
        /// Run phát sự kiện.
        run_id: RunId,
        /// Nội dung text.
        text: String,
    },
    /// Một phần văn bản mới sinh; client cùng run phải nối theo thứ tự nhận.
    TextDelta {
        /// Session của run.
        session_id: SessionId,
        /// Run phát sự kiện.
        run_id: RunId,
        /// Phần text mới, chưa phải snapshot đầy đủ.
        text: String,
        /// Delta index tăng trong một lượt LLM.
        index: u32,
        /// `true` khi text phải thay snapshot cũ thay vì nối vào nó.
        reset: bool,
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
    /// Metadata tùy chọn để adapter hiển thị hành động đúng cấu trúc.
    #[serde(default)]
    pub action: Option<OutboundAction>,
}

/// Hành động gắn với outbound chủ động.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundAction {
    /// Đề xuất skill nháp cần người dùng duyệt.
    SkillDraft {
        /// ID draft ngẫu nhiên.
        id: String,
        /// Tên skill được đề xuất.
        name: String,
        /// User ID đã tạo run; callback inline chỉ nhận đúng user này.
        actor: String,
    },
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

/// Mức nghiêm trọng của một cảnh báo (M23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    /// Thông tin, không cần hành động.
    Low,
    /// Cần xem nhưng không gấp.
    Medium,
    /// Cần người quản trị xem **ngay**.
    High,
}

impl AlertSeverity {
    /// Cảnh báo mức cao có đáng gửi thẳng cho kênh chính không?
    ///
    /// `Plan.md` M23 yêu cầu cảnh báo mức cao gửi **thẳng** cho chủ dự án, song song với
    /// báo cáo chuẩn hoá gửi Manager. Mức thấp/trung bình chỉ nằm trong báo cáo.
    #[must_use]
    pub const fn needs_direct_alert(self) -> bool {
        matches!(self, Self::High)
    }
}

/// Cảnh báo chủ động gửi **thẳng** cho kênh chính, tách khỏi báo cáo gửi Manager (M23).
///
/// Tuần tự hoá được (ràng buộc `Plan.md` mục 4.1) để sẵn sàng cho kiến trúc B.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    /// Mức nghiêm trọng.
    pub severity: AlertSeverity,
    /// Tiêu đề ngắn, ví dụ "Cảnh báo quét bảo mật".
    pub title: String,
    /// Tóm tắt một dòng cho người đọc nhanh.
    pub summary: String,
    /// Các rủi ro chi tiết.
    pub risks: Vec<String>,
}
