//! Lỗi của tool (agents.md mục 6, 7).
//!
//! Mọi lỗi tool đều **không được** làm hỏng vòng lặp agent: agent loop biến chúng thành
//! message `Tool` với `is_error = true` để model đọc và tự sửa. Vì vậy chuỗi lỗi phải rõ
//! ràng, nêu được *cái gì sai* và *nên sửa thế nào*.

/// Lỗi khi đăng ký hoặc thực thi tool.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Đường dẫn không tồn tại.
    #[error("không tìm thấy: {0}")]
    NotFound(String),

    /// Đường dẫn tuyệt đối, chứa `..` hoặc thoát ra ngoài workspace.
    ///
    /// M3 chỉ kiểm tra đơn giản; M4 thay bằng path jail `cap-std` đầy đủ
    /// (agents.md mục 15.1) — cài đặt nằm sau [`crate::WorkspaceFs`].
    #[error("đường dẫn không hợp lệ hoặc nằm ngoài workspace: {0}")]
    NotInWorkspace(String),

    /// Tham số không khớp schema (thiếu, thừa, sai kiểu) — kèm gợi ý tham số hợp lệ.
    #[error("tham số không hợp lệ: {0}")]
    InvalidArgs(String),

    /// Nội dung không phải văn bản UTF-8 (file nhị phân).
    #[error("không đọc được dưới dạng văn bản (file nhị phân?): {0}")]
    InvalidData(String),

    /// Lỗi hệ thống tập tin khác.
    #[error("lỗi I/O: {0}")]
    Io(String),

    /// Tool vượt quá thời gian cho phép (timeout do agent loop áp, mục 6).
    #[error("tool vượt quá thời gian cho phép ({0}s)")]
    Timeout(u64),
    /// Lỗi giao tiếp với MCP server (kết nối, discovery, timeout hoặc kết quả `isError`).
    /// Payload từ server vẫn được bọc `<untrusted_content>` trước khi đi vào thông báo lỗi.
    #[error("lỗi MCP: {0}")]
    Mcp(String),

    /// Đăng ký hai tool trùng tên.
    #[error("tool đã được đăng ký: {0}")]
    DuplicateName(String),

    /// Lỗi nội bộ (ví dụ `spawn_blocking` bị JoinError) — không phải lỗi của model.
    #[error("lỗi nội bộ: {0}")]
    Internal(String),
}
