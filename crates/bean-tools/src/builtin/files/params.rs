//! Tham số cho từng tool file (agents.md mục 7.3).
//!
//! Mỗi struct tuân `JsonSchema` (schemars): doc comment thành `description` gửi model,
//! `#[serde(deny_unknown_fields)]` ⇒ tham số thừa bị từ chối.

use schemars::JsonSchema;
use serde::Deserialize;

/// Đọc file trong workspace.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Đọc nội dung file trong workspace.
///
/// Chỉ dùng đường dẫn tương đối với thư mục gốc workspace (không tuyệt đối, không `..`).
/// Sử dụng `offset`/`limit` để đọc từng phần nếu file dài.
pub struct ReadFileParams {
    /// Đường dẫn tương đối với workspace, ví dụ `src/main.rs`.
    pub path: String,
    /// Bỏ qua N byte đầu; mặc định 0.
    #[serde(default)]
    pub offset: u64,
    /// LimER số ký tự trả về; mặc định 8.000.
    #[serde(default = "default_read_limit")]
    pub limit: u64,
}

fn default_read_limit() -> u64 {
    8000
}

/// Liệt kê thư mục (không đệ quy).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Liệt kê tên file/thư mục trong một thư mục của workspace.
///
/// Không đệ quy: chỉ trả direct children. Đường dẫn rỗng = root workspace.
pub struct ListDirParams {
    /// Thư mục cần liệt kê (tương đối với workspace); rỗng = root.
    #[serde(default)]
    pub path: String,
}

/// Tìm file theo glob.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Tìm các file khớp mẫu glob trong cây workspace.
///
/// `pattern` dùng cú pháp glob (`*` trong cùng thư mục, `**` duyệt đệ quy). Kết quả là
/// danh sách đường dẫn tương đối.
pub struct GlobParams {
    /// Mẫu glob, ví dụ `src/**/*.rs` hoặc `*.md`.
    pub pattern: String,
    /// LimER số kết quả; mặc định 200.
    #[serde(default = "default_glob_limit")]
    pub limit: usize,
}

fn default_glob_limit() -> usize {
    200
}

/// Tìm regex trong file văn bản.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Tìm dòng khớp regex trong các file văn bản của workspace.
///
/// `pattern` là regex Rust. Bỏ qua file nhị phân. Trả danh sách dòng khớp kèm đường dẫn
/// và số dòng.
pub struct GrepParams {
    /// Biểu thức chính quy (regex Rust).
    pub pattern: String,
    /// File/thư mục cần tìm (tương đối); `null` = toàn bộ workspace.
    #[serde(default)]
    pub rel: Option<String>,
    /// LimER số dòng khớp; mặc định 100.
    #[serde(default)]
    pub limit: usize,
}

/// Ghi file trong workspace.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Ghi (tạo hoặc thay thế) file trong workspace.
///
/// Nội dung là văn bản thuần. Tự tạo thư mục cha nếu cần. Chỉ làm việc trong workspace.
pub struct WriteFileParams {
    /// Đường dẫn tương đối với workspace.
    pub path: String,
    /// Nội dung văn bản ghi vào file.
    pub content: String,
}

/// Sửa file (thay chuỗi duy nhất).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
/// Thay một lần xuất hiện duy nhất của `old` bằng `new` trong file workspace.
///
/// `old` phải xuất hiện đúng 1 lần; nếu không, tool báo lỗi để model điều chỉnh.
pub struct EditFileParams {
    /// Đường dẫn tương đối với workspace.
    pub path: String,
    /// Chuỗi hiện tại cần thay — phải duy nhất trong file.
    pub old: String,
    /// Chuỗi thay thế.
    pub new: String,
}
