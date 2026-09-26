//! Implement tool cho nhóm file (agents.md mục 7.3).
//!
//! Mọi I/O đều đi qua [`crate::WorkspaceFs`] (M4: `CapWorkspace` trên cap-std) —
//! không có thao tác `std::fs` trực tiếp nào trong đây.
//!
//! **Nội dung không tin cậy (mục 15.4):** `read_file`, `grep`, `glob`, `list_dir` đều
//! trả văn bản mà kẻ tấn công kiểm soát được (file trong workspace), nên cả bốn tool
//! khai báo `.untrusted()` — [`TypedTool::call`] tự bọc `<untrusted_content>` và bật
//! cờ `untrusted_seen` cho cả lượt, khiến mọi tool `Confirm` trở lên phải hỏi lại.
//! `write_file`/`edit_file` chỉ trả về thông báo do chính agent tạo ra nên không bọc.
use std::sync::Arc;

use beanagent_types::Risk;

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;
use crate::typed::TypedTool;
use crate::workspace::WorkspaceFs;

use super::params::*;

/// Tool đọc file (Safe).
pub fn read_file() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "read_file",
            Risk::Safe,
            |ctx: &ToolCtx, p: ReadFileParams| {
                let ws = ctx.workspace.clone();
                let path = p.path.clone();
                let offset = p.offset;
                let limit = p.limit;
                async move { read_capped(ws, &path, offset, limit).await }
            },
        )
        .untrusted(),
    )
}

async fn read_capped(
    ws: Arc<dyn WorkspaceFs>,
    rel: &str,
    offset: u64,
    limit: u64,
) -> Result<String, ToolError> {
    let n = limit as usize;
    let ws2 = ws.clone();
    let rel_for_closure = rel.to_string();
    // Từ M4: đọc qua capability (`WorkspaceFs::read_range`) — không còn `std::fs`
    // với đường dẫn tự nối, nên symlink thoát workspace cũng bị chặn (mục 15.1).
    let buf =
        tokio::task::spawn_blocking(move || ws2.read_range(&rel_for_closure, offset, n as u64))
            .await
            .map_err(|e| ToolError::Internal(e.to_string()))??;
    let text = String::from_utf8(buf).map_err(|_| ToolError::InvalidData(rel.to_string()))?;
    if crate::text::truncate_chars(&text, n).is_some() {
        Ok(format!("{text}[đã cắt — dùng offset để đọc tiếp]"))
    } else {
        Ok(text)
    }
}

/// Tool liệt kê thư mục (Safe).
pub fn list_dir() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new("list_dir", Risk::Safe, |ctx: &ToolCtx, p: ListDirParams| {
            let ws = ctx.workspace.clone();
            let path = p.path.clone();
            async move {
                let rel = if path.trim().is_empty() { "." } else { &path };
                let entries = ws.list_dir(rel)?;
                serde_json::to_string(&entries).map_err(|e| ToolError::Internal(e.to_string()))
            }
        })
        .untrusted(),
    )
}

/// Tool tìm file theo glob (Safe).
pub fn glob() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new("glob", Risk::Safe, |ctx: &ToolCtx, p: GlobParams| {
            let ws = ctx.workspace.clone();
            let pattern = p.pattern.clone();
            let limit = p.limit;
            async move {
                let paths = ws.glob(&pattern, limit)?;
                serde_json::to_string(&paths).map_err(|e| ToolError::Internal(e.to_string()))
            }
        })
        .untrusted(),
    )
}

/// Tool tìm regex (Safe).
pub fn grep() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new("grep", Risk::Safe, |ctx: &ToolCtx, p: GrepParams| {
            let ws = ctx.workspace.clone();
            let pattern = p.pattern.clone();
            let rel = p.rel.clone();
            let limit = p.limit;
            async move {
                let matches = ws.grep(&pattern, rel.as_deref(), limit)?;
                serde_json::to_string(&matches).map_err(|e| ToolError::Internal(e.to_string()))
            }
        })
        .untrusted(),
    )
}

/// Tool ghi file (Confirm).
///
/// **RBAC (M21.6):** yêu cầu tag `dev-write` — đây là quyền *sửa code dự án*, nên role `qa`
/// (chỉ có `dev-read`/`test-run`) không thấy tool này. Nhờ vậy nguyên tắc four-eyes được
/// ràng buộc **ở tầng code**: agent review không có công cụ để tự sửa code nó đang review.
pub fn write_file() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "write_file",
            Risk::Confirm,
            |ctx: &ToolCtx, p: WriteFileParams| {
                let ws = ctx.workspace.clone();
                let path = p.path.clone();
                let content = p.content.clone();
                async move {
                    ws.write_text(&path, &content)?;
                    Ok(format!("đã ghi `{}` — {} byte", path, content.len()))
                }
            },
        )
        .requires_tags(["dev-write"]),
    )
}

/// Tool sửa file (Confirm).
///
/// **RBAC (M21.6):** yêu cầu tag `dev-write` — xem [`write_file`].
pub fn edit_file() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "edit_file",
            Risk::Confirm,
            |ctx: &ToolCtx, p: EditFileParams| {
                let ws = ctx.workspace.clone();
                let path = p.path.clone();
                let old = p.old.clone();
                let new_str = p.new.clone();
                async move {
                    ws.edit_unique(&path, &old, &new_str)?;
                    Ok(format!(
                        "đã thay `{}` → `{}` trong `{}`",
                        old, new_str, path
                    ))
                }
            },
        )
        .requires_tags(["dev-write"]),
    )
}

// Test đọc/ghi/offset/grep của nhóm file nằm ở `beanagent-security/tests/file_tools.rs`
// (dùng `CapWorkspace` thật) — tool crate không thể phụ thuộc security (vòng phụ thuộc).
