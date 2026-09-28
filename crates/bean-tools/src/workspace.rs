//! Truy cập file bị jail trong workspace (agents.md mục 15.1, 7.1).
//!
//! Từ **M4**, trait này được cài duy nhất bởi [`bean_security::paths::CapWorkspace`]
//! (`cap_std::fs::Dir` gốc workspace — chặn đường dẫn tuyệt đối, `..` và symlink thoát ra).
//! M3 đã xoá cài đặt tạm thời `FsWorkspace`; mọi tool builtin chỉ gọi qua trait nên việc
//! thay cài đặt không đụng tool.
//!
//! Mọi hàm là **blocking**; caller async phải bọc `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::error::ToolError;

/// Một dòng khớp của `grep`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GrepMatch {
    /// Đường dẫn tương đối với workspace (dấu `/`).
    pub path: String,
    /// Số dòng (1-based).
    pub line_no: usize,
    /// Nội dung dòng (đã cắt nếu quá dài).
    pub line: String,
}

/// Mục trong danh sách thư mục của `list_dir`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DirEntryInfo {
    /// Tên (không kèm đường dẫn).
    pub name: String,
    /// Thư mục hay file?
    pub is_dir: bool,
    /// Kích thước byte (file; thư mục = 0).
    pub size: u64,
}

/// Capability truy cập file trong workspace — M4 cài bằng `cap_std::fs::Dir`
/// ([`bean_security::paths::CapWorkspace`]); mọi tool chỉ gọi qua trait này.
pub trait WorkspaceFs: Send + Sync + std::fmt::Debug {
    /// Đường dẫn gốc tuyệt đối (để hiển thị, không dùng để đọc file trực tiếp).
    fn root(&self) -> &Path;

    /// Kiểm tra và chuẩn hoá đường dẫn tương đối; lỗi [`ToolError::NotInWorkspace`]
    /// khi tuyệt đối, chứa `..`, hoặc rỗng.
    ///
    /// Kết quả chỉ dùng để **hiển thị/log** — mọi đọc/ghi thực sự đi qua các method
    /// dưới đây nên không thể thoát khỏi sandbox kể cả qua symlink.
    fn resolve(&self, rel: &str) -> Result<PathBuf, ToolError>;

    /// Đọc tối đa `limit` byte bắt đầu từ `offset` (byte) của file — dùng cho
    /// `read_file` với `offset`/`limit` để đọc tiếp khi kết quả dài.
    fn read_range(&self, rel: &str, offset: u64, limit: u64) -> Result<Vec<u8>, ToolError>;

    /// Đọc file thành văn bản (lỗi [`ToolError::InvalidData`] nếu không phải UTF-8).
    fn read_text(&self, rel: &str) -> Result<String, ToolError>;

    /// Ghi (tạo/thay thế) file văn bản; tự tạo thư mục cha.
    fn write_text(&self, rel: &str, content: &str) -> Result<(), ToolError>;

    /// Thay thế chuỗi **duy nhất** trong file (`edit_file`, mục 7.3).
    ///
    /// Trả lỗi khi `old` không xuất hiện hoặc xuất hiện nhiều hơn một lần — model phải
    /// cung cấp chuỗi đủ dài để duy nhất.
    fn edit_unique(&self, rel: &str, old: &str, new: &str) -> Result<(), ToolError>;

    /// Liệt kê thư mục (không đệ quy).
    fn list_dir(&self, rel: &str) -> Result<Vec<DirEntryInfo>, ToolError>;

    /// Duyệt đệ quy và trả các đường dẫn (tương đối, dấu `/`) khớp glob.
    /// `pattern` rỗng ⇒ trả rỗng.
    fn glob(&self, pattern: &str, limit: usize) -> Result<Vec<String>, ToolError>;

    /// Tìm regex trong các file văn bản (duyệt đệ quy, bỏ file nhị phân).
    /// `rel = None` ⇒ quét cả workspace; `rel` trỏ tới file ⇒ chỉ file đó; trỏ tới
    /// thư mục ⇒ quét cây con.
    fn grep(
        &self,
        pattern: &str,
        rel: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GrepMatch>, ToolError>;
}
// Test của WorkspaceFs nằm ở crate bean-security (CapWorkspace) —
// mọi tool builtin đều chỉ gọi qua trait này.
