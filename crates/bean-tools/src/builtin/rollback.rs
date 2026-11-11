//! Tool `rollback_workspace` (D26) — hoàn tác workspace về checkpoint của lượt.
//!
//! # Vì sao tồn tại
//!
//! Model có thể ghi nhầm file, xoá nhầm nội dung trong lúc làm việc dài. Checkpoint
//! được agent loop chụp **trước tool ghi đầu tiên** của lượt (chỉ khi checkpoint bật),
//! nên rollback luôn trả workspace về đúng trạng thái ngay trước khi model bắt đầu
//! thay đổi — điểm an toàn duy nhất mà mọi lượt đều có.
//!
//! # Bất biến
//!
//! * **`Confirm`** — xoá/ghi đè file là hành động phá huỷ, luôn hỏi người dùng và
//!   **không** có tuỳ chọn "cho phép trong phiên" (nay là mức cao nhất sau Dangerous);
//!   chạy dưới scheduler (không có người duyệt) bị từ chối tự động theo quy tắc
//!   bốn mắt (agents.md mục 14).
//! * Thẻ xác nhận hiển thị **danh sách thay đổi thật** (đo tại thời điểm hỏi) do
//!   agent loop bổ sung vào prompt — model không thể che giấu những gì sẽ bị hoàn tác.
//! * Chỉ khôi phục về checkpoint **của lượt hiện tại**: nhắm giữ phạm vi nhỏ, không
//!   trờ thành máy thời gian nhiều tầng (mở rộng sau nếu cần).

use std::path::PathBuf;
use std::sync::Arc;

use bean_types::Risk;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::checkpoint::ROLLBACK_TOOL;
use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;
use crate::typed::TypedTool;

/// Tham số `rollback_workspace` — không có tham số nào: mục tiêu khôi phục luôn là
/// checkpoint của lượt hiện tại, agent loop quyết định, model không chỉ đường được.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RollbackParams {}

/// Hoàn tác mọi thay đổi workspace của lượt hiện tại, trả về trạng thái ngay trước
/// khi lượt bắt đầu ghi.
///
/// Dùng khi bạn nhận ra đã ghi nhầm file hoặc xoá nhầm nội dung và muốn quay lại
/// trạng thái trước đó của lượt này. Người dùng sẽ được hỏi xác nhận kèm danh sách
/// thay đổi sẽ bị hoàn tác.
#[must_use]
pub fn tool() -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
        ROLLBACK_TOOL,
        Risk::Confirm,
        |ctx: &ToolCtx, _params: RollbackParams| {
            let checkpoint = ctx.checkpoint.clone();
            let workspace_root: Option<PathBuf> = ctx.workspace.root().to_path_buf().into();
            async move {
                let Some(ckpt) = checkpoint else {
                    return Err(ToolError::InvalidArgs(
                        "Lượt này chưa có checkpoint — chưa có tool nào thay đổi workspace \
                         để hoàn tác. Chỉ gọi sau khi đã ghi/sửa file trong lượt hiện tại."
                            .into(),
                    ));
                };
                let Some(root) = workspace_root else {
                    return Err(ToolError::InvalidArgs(
                        "Không xác định được thư mục gốc của workspace.".into(),
                    ));
                };
                let report = ckpt.restore(&root).await?;
                Ok(format!(
                    "Đã khôi phục workspace về {}: {} file được trả về bản cũ, {} file sinh sau checkpoint đã bị xoá.",
                    ckpt.label(),
                    report.restored,
                    report.removed
                ))
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::{Path, PathBuf};

    use async_trait::async_trait;
    use bean_types::SessionId;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::checkpoint::{ChangeList, WorkspaceCheckpoint};
    use crate::ctx::ToolCtx;
    use crate::workspace::WorkspaceFs;

    /// Workspace giả: rollback chỉ cần `root()`; mọi thao tác file đều lỗi.
    #[derive(Debug)]
    struct Fs(PathBuf);

    impl WorkspaceFs for Fs {
        fn root(&self) -> &Path {
            &self.0
        }

        fn resolve(&self, rel: &str) -> Result<PathBuf, ToolError> {
            Ok(PathBuf::from(rel))
        }

        fn read_range(&self, _rel: &str, _offset: u64, _limit: u64) -> Result<Vec<u8>, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn read_text(&self, _rel: &str) -> Result<String, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn write_text(&self, _rel: &str, _text: &str) -> Result<(), ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn edit_unique(&self, _rel: &str, _old: &str, _new: &str) -> Result<(), ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn list_dir(&self, _rel: &str) -> Result<Vec<crate::DirEntryInfo>, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn glob(&self, _pattern: &str, _limit: usize) -> Result<Vec<String>, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn grep(
            &self,
            _pattern: &str,
            _rel: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<crate::GrepMatch>, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }
    }

    /// Cài đặt giả: đếm số lần restore, trả báo cáo đặt sẵn.
    struct FakeCheckpoint {
        restores: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl WorkspaceCheckpoint for FakeCheckpoint {
        fn label(&self) -> String {
            "checkpoint TEST".into()
        }
        async fn changes(&self, _root: &Path) -> Result<ChangeList, ToolError> {
            Ok(ChangeList {
                added: vec!["new.txt".into()],
                ..ChangeList::default()
            })
        }
        async fn restore(
            &self,
            _root: &Path,
        ) -> Result<crate::checkpoint::RestoreReport, ToolError> {
            self.restores
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::checkpoint::RestoreReport {
                restored: 2,
                removed: 1,
            })
        }
    }

    fn ctx_with(checkpoint: Option<Arc<dyn WorkspaceCheckpoint>>) -> ToolCtx {
        let dir = tempfile::tempdir().expect("tempdir");
        ToolCtx {
            workspace: Arc::new(Fs(dir.path().to_path_buf())),
            session: SessionId::new(1),
            cancel: CancellationToken::new(),
            untrusted_seen: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            project: "default".into(),
            alerts: None,
            checkpoint,
        }
    }

    #[tokio::test]
    async fn khong_co_checkpoint_thi_tra_loi_ro_rang() {
        let tool = tool();
        let ctx = ctx_with(None);
        let error = tool
            .call(&ctx, serde_json::json!({}))
            .await
            .expect_err("phải lỗi khi chưa có checkpoint");
        assert!(error.to_string().contains("chưa có checkpoint"), "{error}");
    }

    #[tokio::test]
    async fn co_checkpoint_thi_khoi_phuc_va_bao_cao() {
        let fake = Arc::new(FakeCheckpoint {
            restores: std::sync::atomic::AtomicUsize::new(0),
        });
        let tool = tool();
        let ctx = ctx_with(Some(fake.clone() as Arc<dyn WorkspaceCheckpoint>));
        let output = tool.call(&ctx, serde_json::json!({})).await.expect("ok");
        assert!(output.contains("2 file được trả về bản cũ"), "{output}");
        assert!(output.contains("1 file sinh sau checkpoint"), "{output}");
        assert_eq!(fake.restores.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn risk_la_confirm_va_ten_dung() {
        let tool = tool();
        assert_eq!(tool.spec().name, "rollback_workspace");
        assert_eq!(tool.risk(&serde_json::json!({})), Risk::Confirm);
    }

    /// Tham số thừa phải bị từ chối (`deny_unknown_fields`).
    #[tokio::test]
    async fn tham_so_thua_bi_tu_choi() {
        let tool = tool();
        let ctx = ctx_with(None);
        let error = tool
            .call(&ctx, serde_json::json!({ "target": "/etc" }))
            .await
            .expect_err("phải từ chối tham số thừa");
        assert!(matches!(error, ToolError::InvalidArgs(_)), "{error}");
    }
}
