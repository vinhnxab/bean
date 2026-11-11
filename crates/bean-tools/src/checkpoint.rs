//! Checkpoint workspace của một lượt (D26) — hợp đồng trừu tượng cho tool
//! `rollback_workspace`.
//!
//! Trait nằm ở `bean-tools` còn cài đặt (`CheckpointStore`) nằm ở `bean-security` —
//! cùng mô hình hướng phụ thuộc với [`WorkspaceFs`](crate::WorkspaceFs): tầng tool
//! không biết gì về `std::fs` của cài đặt.
//!
//! # Bất biến an toàn
//!
//! * snapshot nằm **ngoài** jail workspace (`data.dir/checkpoints/`) nên model không
//!   đọc được qua `read_file` và không thể tự xoá dấu vết của mình;
//! * đường dẫn workspace/snapshot đều đến từ **cấu hình vận hành** — không bao giờ
//!   từ model; module này chỉ nhận nhãn và trả báo cáo.

use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ToolError;

/// Tên tool khôi phục workspace về checkpoint của lượt hiện tại.
pub const ROLLBACK_TOOL: &str = "rollback_workspace";

/// Số dòng tối đa mỗi nhóm thay đổi khi rút gọn thành văn bản cho thẻ xác nhận.
const MAX_SUMMARY_LINES: usize = 20;

/// Danh sách thay đổi giữa snapshot và workspace hiện tại.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeList {
    /// File có trong workspace nhưng chưa có trong snapshot (sinh sau checkpoint).
    pub added: Vec<String>,
    /// File có ở cả hai nhưng khác nội dung (so kích thước/mtime — heuristic đủ
    /// cho thẻ xác nhận; khôi phục luôn ghi đè nội dung thật).
    pub modified: Vec<String>,
    /// File có trong snapshot nhưng đã bị xoá khỏi workspace.
    pub removed: Vec<String>,
}

impl ChangeList {
    /// `true` khi không có thay đổi nào.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.removed.is_empty()
    }

    /// Rút gọn thành văn bản tiếng Việt liệt kê thay đổi (mỗi nhóm tối đa
    /// [`MAX_SUMMARY_LINES`] dòng, phần thừa gộp thành "… và N file nữa").
    #[must_use]
    pub fn summary(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut push_group = |label: &str, files: &[String]| {
            if files.is_empty() {
                return;
            }
            for name in files.iter().take(MAX_SUMMARY_LINES) {
                lines.push(format!("{label} {name}"));
            }
            let rest = files.len().saturating_sub(MAX_SUMMARY_LINES);
            if rest > 0 {
                lines.push(format!("… và {rest} file nữa cùng loại"));
            }
        };
        push_group("thêm:", &self.added);
        push_group("sửa:", &self.modified);
        push_group("xoá:", &self.removed);
        lines.join("\n")
    }
}

/// Báo cáo sau khi khôi phục workspace về snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// Số file đã ghi lại từ snapshot (bao gồm cả file "sửa" được trả về bản cũ).
    pub restored: usize,
    /// Số file sinh sau checkpoint đã bị xoá.
    pub removed: usize,
}

/// Checkpoint của lượt hiện tại — cài đặt trong `bean-security::checkpoint`.
#[async_trait]
pub trait WorkspaceCheckpoint: Send + Sync {
    /// Nhãn hiển thị trong thẻ xác nhận (ví dụ `checkpoint 20261110-123458-ab12cd34`).
    fn label(&self) -> String;

    /// So sánh snapshot với workspace hiện tại.
    ///
    /// # Errors
    /// [`ToolError::Io`] khi không đọc được snapshot hay workspace.
    async fn changes(&self, workspace_root: &Path) -> Result<ChangeList, ToolError>;

    /// Khôi phục workspace về snapshot: ghi đè từ snapshot trước, rồi xoá các file
    /// sinh sau checkpoint, cuối cùng dọn thư mục rỗng. Gián đoạn giữa chừng để lại
    /// workspace nửa vằng vặt nhưng snapshot còn nguyên — chạy lại được.
    ///
    /// # Errors
    /// [`ToolError::Io`] khi không đọc/ghi được.
    async fn restore(&self, workspace_root: &Path) -> Result<RestoreReport, ToolError>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn summary_gom_theo_nhom_va_cat_tran() {
        let changes = ChangeList {
            added: (0..25).map(|i| format!("new-{i}.txt")).collect(),
            modified: vec!["a.md".into()],
            removed: vec![],
        };
        let text = changes.summary();
        assert_eq!(text.matches("thêm: ").count(), MAX_SUMMARY_LINES);
        assert!(text.contains("… và 5 file nữa cùng loại"));
        assert!(text.contains("sửa: a.md"));
        assert!(!text.contains("xoá:"));
    }

    #[test]
    fn summary_rong_khi_khong_thay_doi() {
        assert!(ChangeList::default().summary().is_empty());
        assert!(ChangeList::default().is_empty());
    }
}
