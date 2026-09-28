//! Ngữ cảnh thực thi tool (agents.md mục 7.1).
//!
//! `ToolCtx` là thứ agent loop truyền vào mỗi lần `Tool::call`. Từ M3 nó chứa:
//! workspace (dạng trait [`crate::WorkspaceFs`], M4 thay bằng cap-std), id phiên,
//! `CancellationToken` và cờ `untrusted_seen` của lượt hiện tại (dùng từ M7 — web/MCP).
//! Handle tới store (memory/memory_search) sẽ được thêm ở M5 khi có tool bộ nhớ.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use bean_types::Alert;
use tokio_util::sync::CancellationToken;

use crate::workspace::WorkspaceFs;
use bean_types::SessionId;

/// Kênh gửi cảnh báo chủ động cho tool (M23).
///
/// Đặt **trait** ở đây (không phải ở `bean-core`) để tool không phụ thuộc Router:
/// `bean-scan` cần gửi cảnh báo, nhưng Router lại điều phối tool — nếu tham chiếu thẳng
/// `Router` sẽ thành phụ thuộc vòng. M23 chỉ cần một trait nhỏ, không cần toàn bộ Router.
///
/// Cài bản thật trong `bean-core` bọc quanh [`Router::notify`], nên lỗi gửi vẫn đi qua
/// outbox như mọi outbound khác (không mất tin).
#[async_trait]
pub trait AlertSink: Send + Sync {
    /// Gửi cảnh báo tới kênh chính. `Err` chỉ để ghi log — **không** được làm hỏng tool.
    async fn send_alert(&self, alert: &Alert) -> Result<(), String>;
}

/// Project profile dùng khi lượt không khai báo project (M21.1) — trùng
/// [`bean_types::config::DEFAULT_PROJECT`], chỉ lặp lại chuỗi để tránh phụ thuộc vòng.
pub const DEFAULT_PROJECT: &str = "default";

/// Ngữ cảnh truyền cho tool khi thực thi.
#[derive(Clone)]
pub struct ToolCtx {
    /// Truy cập file bị jail trong workspace (M4: `CapWorkspace` — cap-std).
    pub workspace: Arc<dyn WorkspaceFs>,
    /// Phiên hội thoại đang chạy (dùng cho log/audit từ M4).
    pub session: SessionId,
    /// Huỷ run giữa chừng: tool nên kiểm tra trong các vòng lặp dài (mục 6).
    pub cancel: CancellationToken,
    /// `true` khi model đã đọc nội dung untrusted trong lượt này — M7 sẽ dùng nó để
    /// vô hiệu hoá "cho phép trong phiên" (mục 15.4). M3: luôn `false`.
    pub untrusted_seen: Arc<AtomicBool>,
    /// Tên project profile của lượt này (M21.1) — dùng để tra bảng tool cần thêm tag
    /// theo project (ví dụ `marketing-draft` chỉ ghi trong workspace của role marketing).
    ///
    /// Không dùng để quyết định quyền: quyền đã được Router resolve trước và truyền xuống
    /// dưới dạng [`bean_types::RolePermissions`] (ràng buộc `Plan.md` mục 4.3).
    pub project: String,
    /// Kênh gửi cảnh báo chủ động (M23); `None` ⇒ tool chỉ trả cảnh báo trong kết quả.
    pub alerts: Option<Arc<dyn AlertSink>>,
}

impl ToolCtx {
    /// Dựng `ToolCtx` cho project `default` — đường ngắn cho test và adapter đơn project.
    #[must_use]
    pub fn for_project(
        workspace: Arc<dyn WorkspaceFs>,
        session: SessionId,
        cancel: CancellationToken,
        untrusted_seen: Arc<AtomicBool>,
    ) -> Self {
        Self {
            workspace,
            session,
            cancel,
            untrusted_seen,
            project: DEFAULT_PROJECT.to_string(),
            alerts: None,
        }
    }

    /// Gắn kênh gửi cảnh báo (M23).
    #[must_use]
    pub fn with_alerts(mut self, alerts: Arc<dyn AlertSink>) -> Self {
        self.alerts = Some(alerts);
        self
    }

    /// Gắn kênh cảnh báo nếu có (M25: đường MCP dùng để `ToolCtx` có cùng khả năng
    /// gửi cảnh báo chủ động như run chat).
    #[must_use]
    pub fn with_alerts_opt(mut self, alerts: Option<Arc<dyn AlertSink>>) -> Self {
        self.alerts = alerts;
        self
    }

    /// Đổi project profile (M21.1).
    #[must_use]
    pub fn with_project_name(mut self, project: impl Into<String>) -> Self {
        self.project = project.into();
        self
    }
}
