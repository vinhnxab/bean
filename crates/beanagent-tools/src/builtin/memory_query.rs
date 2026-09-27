//! Tool `memory_query` (M25) — đọc `MEMORY.md`/`USER.md`, **không** cho sửa.
//!
//! # Vì sao tool này tồn tại
//!
//! `Plan.md` M25 yêu cầu đường MCP server expose "một tool memory-query (đọc
//! `MEMORY.md`/`USER.md` hiện có, KHÔNG cho sửa)". Đó là hai file mà
//! `beanagent_core::context` nạp vào system prompt (mục 8.2) — nhờ đó agent khác
//! (Cline, Cursor…) biết được bối cảnh dài hạn của người dùng mà **không** phải đọc
//! file tùy ý.
//!
//! # Bất biến
//!
//! * **`Safe`** — chỉ đọc, không có tham số nào để điều khiển ghi
//!   (`#[serde(deny_unknown_fields)]` ⇒ tham số thừa bị từ chối).
//! * Chỉ hai tên file được phép; **không** nhận đường dẫn tùy ý ⇒ không biến thành
//!   đường đọc file tùy ý trong workspace (đường đọc file hợp lệ đã có `read_file`).
//! * Tag `memory-read` (M25) — một trong ba tag được MCP server expose.
//! * Nội dung file là dữ liệu **ngoài lõi** (file trong workspace, mục 15.4) nên bọc
//!   `<untrusted_content>` và bật cờ `untrusted_seen`.

use std::sync::Arc;

use beanagent_types::Risk;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ctx::ToolCtx;
use crate::error::ToolError;
use crate::tool::Tool;
use crate::typed::TypedTool;
use crate::workspace::WorkspaceFs;

/// Trần ký tự trả về cho client MCP.
///
/// `MEMORY.md`/`USER.md` trong system prompt cũng bị cắt ở cùng trần
/// (`MAX_MEMORY_FILE_CHARS` trong `beanagent_core::context`) — hai đường đọc phải
/// cho cùng một nội dung, nếu không client MCP và agent sẽ "thấy" hai sự thật khác nhau.
pub const MAX_CHARS: usize = 4_000;

/// File nhớ được phép đọc. Không có đường dẫn tùy ý — xem module docs.
const FILES: [(&str, &str); 2] = [("MEMORY", "MEMORY.md"), ("USER", "USER.md")];

/// Đọc ghi chú dài hạn của người dùng (`MEMORY.md` hoặc `USER.md`) trong workspace.
///
/// Dùng khi bạn cần biết người dùng đã lưu quyết định, sở thích hay quy ước nào từ
/// trước, mà không muốn lục lọi cả lịch sử hội thoại. Chỉ đọc — muốn thay đổi ghi chú
/// thì hãy nói với người dùng.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MemoryQueryParams {
    /// File cần đọc: `MEMORY` (ghi chú của agent về dự án/quyết định) hoặc `USER`
    /// (sở thích, quy ước cá nhân của người dùng).
    file: String,
}

/// Tool `memory_query` (Safe, read-only).
#[must_use]
pub fn tool() -> Arc<dyn Tool> {
    Arc::new(
        TypedTool::new(
            "memory_query",
            Risk::Safe,
            |ctx: &ToolCtx, p: MemoryQueryParams| {
                let ws = ctx.workspace.clone();
                let requested = p.file.clone();
                async move { read_memory_file(ws, &requested).await }
            },
        )
        .requires_tags([beanagent_types::config::MEMORY_READ_TAG])
        .untrusted(),
    )
}

async fn read_memory_file(ws: Arc<dyn WorkspaceFs>, requested: &str) -> Result<String, ToolError> {
    let file = FILES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(requested.trim()))
        .map(|(_, path)| (*path).to_string())
        .ok_or_else(|| ToolError::InvalidArgs("`file` phải là `MEMORY` hoặc `USER`".into()))?;
    let ws2 = ws.clone();
    let name_for_closure = file.clone();
    let text = tokio::task::spawn_blocking(move || ws2.read_text(&name_for_closure))
        .await
        .map_err(|e| ToolError::Internal(e.to_string()))??;
    if text.trim().is_empty() {
        return Ok(format!(
            "{file} hiện trống — người dùng chưa ghi gì vào đây."
        ));
    }
    Ok(
        crate::text::truncate_chars(&text, MAX_CHARS).map_or(text.clone(), |(kept, cut)| {
            format!("{kept}\n[đã cắt {cut} ký tự — file đầy đủ nằm trong workspace]")
        }),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use beanagent_types::SessionId;
    use std::sync::atomic::AtomicBool;
    use tokio_util::sync::CancellationToken;

    /// Workspace giả: chỉ `MEMORY.md` có nội dung, `write_text` luôn lỗi (chứng minh
    /// tool này **không** ghi được gì).
    #[derive(Debug)]
    struct Fs;

    impl WorkspaceFs for Fs {
        fn root(&self) -> &std::path::Path {
            std::path::Path::new(".")
        }

        fn resolve(&self, rel: &str) -> Result<std::path::PathBuf, ToolError> {
            Ok(std::path::PathBuf::from(rel))
        }

        fn read_text(&self, rel: &str) -> Result<String, ToolError> {
            match rel {
                "MEMORY.md" => Ok("dự án Bean: chỉ đọc".to_string()),
                "USER.md" => Ok(String::new()),
                _ => Err(ToolError::NotFound(rel.to_string())),
            }
        }

        fn read_range(&self, rel: &str, offset: u64, limit: u64) -> Result<Vec<u8>, ToolError> {
            let text = self.read_text(rel)?;
            let start = usize::try_from(offset).unwrap_or(0).min(text.len());
            let end = (start + usize::try_from(limit).unwrap_or(0)).min(text.len());
            Ok(text.as_bytes()[start..end].to_vec())
        }

        fn write_text(&self, _rel: &str, _text: &str) -> Result<(), ToolError> {
            Err(ToolError::InvalidData(
                "memory_query không được ghi file".into(),
            ))
        }

        fn edit_unique(&self, _rel: &str, _old: &str, _new: &str) -> Result<(), ToolError> {
            Err(ToolError::InvalidData(
                "memory_query không được sửa file".into(),
            ))
        }

        fn list_dir(&self, _rel: &str) -> Result<Vec<crate::DirEntryInfo>, ToolError> {
            Ok(Vec::new())
        }

        fn glob(&self, _pattern: &str, _limit: usize) -> Result<Vec<String>, ToolError> {
            Ok(Vec::new())
        }

        fn grep(
            &self,
            _pattern: &str,
            _rel: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<crate::GrepMatch>, ToolError> {
            Ok(Vec::new())
        }
    }

    fn ctx() -> ToolCtx {
        ToolCtx::for_project(
            Arc::new(Fs),
            SessionId::new(1),
            CancellationToken::new(),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[tokio::test]
    async fn reads_both_files_but_rejects_arbitrary_paths() {
        let tool = tool();
        let out = tool
            .call(&ctx(), serde_json::json!({"file": "MEMORY"}))
            .await
            .unwrap();
        assert!(out.contains("dự án Bean"));
        // `USER.md` rỗng ⇒ trả thông báo rõ, không phải lỗi.
        let empty = tool
            .call(&ctx(), serde_json::json!({"file": "user"}))
            .await
            .unwrap();
        assert!(empty.contains("trống"));
        // Không có đường dẫn tùy ý: `MEMORY.md`/`USER.md` là toàn bộ bề mặt đọc.
        for bad in ["../etc/passwd", "MEMORY.md", "skills/x/SKILL.md", ""] {
            assert!(
                tool.call(&ctx(), serde_json::json!({"file": bad}))
                    .await
                    .is_err(),
                "`{bad}` phải bị từ chối"
            );
        }
        // Tham số thừa bị từ chối nhờ `deny_unknown_fields` ⇒ không có đường nào ghi.
        assert!(
            tool.call(
                &ctx(),
                serde_json::json!({"file": "MEMORY", "overwrite": true})
            )
            .await
            .is_err()
        );
    }

    #[test]
    fn is_safe_tagged_and_marked_untrusted() {
        let tool = tool();
        assert_eq!(tool.risk(&serde_json::json!({})), Risk::Safe);
        assert_eq!(
            tool.required_tags(),
            vec![beanagent_types::config::MEMORY_READ_TAG]
        );
        // Nội dung file là dữ liệu ngoài lõi (mục 15.4) ⇒ phải báo untrusted.
        assert!(tool.marks_untrusted());
    }
}
