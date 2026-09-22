//! Ngữ cảnh thực thi tool (agents.md mục 7.1).
//!
//! `ToolCtx` là thứ agent loop truyền vào mỗi lần `Tool::call`. Từ M3 nó chứa:
//! workspace (dạng trait [`crate::WorkspaceFs`], M4 thay bằng cap-std), id phiên,
//! `CancellationToken` và cờ `untrusted_seen` của lượt hiện tại (dùng từ M7 — web/MCP).
//! Handle tới store (memory/memory_search) sẽ được thêm ở M5 khi có tool bộ nhớ.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio_util::sync::CancellationToken;

use crate::workspace::WorkspaceFs;
use beanagent_types::SessionId;

/// Ngữ cảnh truyền cho tool khi thực thi.
#[derive(Clone)]
pub struct ToolCtx {
    /// Truy cập file bị jail trong workspace (M3: `FsWorkspace`; M4: cap-std).
    pub workspace: Arc<dyn WorkspaceFs>,
    /// Phiên hội thoại đang chạy (dùng cho log/audit từ M4).
    pub session: SessionId,
    /// Huỷ run giữa chừng: tool nên kiểm tra trong các vòng lặp dài (mục 6).
    pub cancel: CancellationToken,
    /// `true` khi model đã đọc nội dung untrusted trong lượt này — M7 sẽ dùng nó để
    /// vô hiệu hoá "cho phép trong phiên" (mục 15.4). M3: luôn `false`.
    pub untrusted_seen: Arc<AtomicBool>,
}
