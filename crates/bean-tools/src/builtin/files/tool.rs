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

use bean_types::Risk;
use serde_json::Value;

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
///
/// **D27:** thẻ xác nhận kèm **diff thật** giữa nội dung hiện có và nội dung sẽ ghi —
/// người dùng duyệt nhanh mà vẫn thấy rõ thay đổi thay vì đọc cả content trong preview.
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
        .requires_tags(["dev-write"])
        .confirm_details_fn(write_file_details),
    )
}

/// (D27) Chi tiết thẻ xác nhận của `write_file`: diff đo **tại thời điểm hỏi** —
/// file có thể đã khác từ lúc model sinh args. Lỗi đo (nhị phân, quá trần…) hiện
/// note thay vì `None` khi biết rõ ý định (ghi đè/tạo mới); lỗi khác trả `None`
/// để agent loop rơi về preview `k=v` mặc định.
fn write_file_details(workspace: &dyn WorkspaceFs, args: &Value) -> Option<String> {
    let path = args.get("path")?.as_str()?;
    let content = args.get("content")?.as_str()?;
    let new_lines = content.lines().count();
    match workspace.read_text(path) {
        Ok(old) => {
            let old_lines = old.lines().count();
            let diff = crate::diff::unified_diff_bounded(
                &old,
                content,
                path,
                crate::diff::DEFAULT_MAX_LINES,
            );
            Some(format!(
                "path={path} — ghi đè ({old_lines} dòng → {new_lines} dòng)\n{diff}"
            ))
        }
        Err(ToolError::NotFound(_)) => {
            let diff = crate::diff::unified_diff_bounded(
                "",
                content,
                path,
                crate::diff::DEFAULT_MAX_LINES,
            );
            Some(format!(
                "path={path} — tạo file mới ({new_lines} dòng)\n{diff}"
            ))
        }
        Err(ToolError::InvalidData(_)) => Some(format!(
            "path={path} — ghi đè ({new_lines} dòng); file cũ là nhị phân nên không hiển thị được diff."
        )),
        Err(_) => None,
    }
}

/// Tool sửa file (Confirm).
///
/// **RBAC (M21.6):** yêu cầu tag `dev-write` — xem [`write_file`].
///
/// **D27:** thẻ xác nhận kèm diff của thay đổi và cảnh báo khi `old` không khớp đúng
/// một chỗ — model biết trước lần gọi này sẽ thất bại, người dùng không duyệt mù.
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
        .requires_tags(["dev-write"])
        .confirm_details_fn(edit_file_details),
    )
}

/// (D27) Chi tiết thẻ xác nhận của `edit_file`. Đếm số lần `old` xuất hiện trong file
/// **tại thời điểm hỏi**: ≠ 1 thì thẻ báo trước lỗi (tool chỉ thay khi đúng một chỗ);
/// = 1 thì mô phỏng `replacen` và hiển thị diff.
fn edit_file_details(workspace: &dyn WorkspaceFs, args: &Value) -> Option<String> {
    let path = args.get("path")?.as_str()?;
    let old = args.get("old")?.as_str()?;
    let new = args.get("new")?.as_str()?;
    let current = workspace.read_text(path).ok()?;
    let occurrences = current.matches(old).count();
    if occurrences != 1 {
        return Some(format!(
            "path={path} — chuỗi cần thay xuất hiện {occurrences} lần (tool chỉ thay khi đúng 1 lần; lần gọi này sẽ báo lỗi)."
        ));
    }
    let after = current.replacen(old, new, 1);
    let diff =
        crate::diff::unified_diff_bounded(&current, &after, path, crate::diff::DEFAULT_MAX_LINES);
    Some(format!("path={path} — thay đúng 1 chỗ\n{diff}"))
}

// Test đọc/ghi/offset/grep của nhóm file nằm ở `bean-security/tests/file_tools.rs`
// (dùng `CapWorkspace` thật) — tool crate không thể phụ thuộc security (vòng phụ thuộc).

#[cfg(test)]
mod confirm_details_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::{edit_file, edit_file_details, write_file, write_file_details};
    use crate::error::ToolError;
    use crate::workspace::{DirEntryInfo, GrepMatch, WorkspaceFs};

    /// Trạng thái file duy nhất mà chi tiết xác nhận cần đọc.
    #[derive(Debug)]
    enum Content {
        /// File tồn tại với nội dung văn bản.
        Text(&'static str),
        /// File tồn tại nhưng không phải UTF-8.
        Binary,
        /// File không tồn tại.
        Missing,
        /// Lỗi I/O khác (vd. file quá trần).
        IoError,
    }

    /// Workspace giả chỉ hỗ trợ `read_text` — mọi method khác lỗi `NotFound`
    /// (pattern `Fs` của `rollback.rs` tests).
    #[derive(Debug)]
    struct Ws(Content);

    impl WorkspaceFs for Ws {
        fn root(&self) -> &Path {
            Path::new("/tmp/test-ws")
        }

        fn resolve(&self, rel: &str) -> Result<PathBuf, ToolError> {
            Ok(PathBuf::from(rel))
        }

        fn read_range(&self, _rel: &str, _offset: u64, _limit: u64) -> Result<Vec<u8>, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn read_text(&self, _rel: &str) -> Result<String, ToolError> {
            match &self.0 {
                Content::Text(text) => Ok((*text).to_string()),
                Content::Binary => Err(ToolError::InvalidData("test".into())),
                Content::Missing => Err(ToolError::NotFound("test".into())),
                Content::IoError => Err(ToolError::Io("quá trần".into())),
            }
        }

        fn write_text(&self, _rel: &str, _text: &str) -> Result<(), ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn edit_unique(&self, _rel: &str, _old: &str, _new: &str) -> Result<(), ToolError> {
            Err(ToolError::NotFound("test".into()))
        }

        fn list_dir(&self, _rel: &str) -> Result<Vec<DirEntryInfo>, ToolError> {
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
        ) -> Result<Vec<GrepMatch>, ToolError> {
            Err(ToolError::NotFound("test".into()))
        }
    }

    #[test]
    fn write_details_reports_new_file() {
        let args = json!({"path": "moi.md", "content": "dòng 1\ndòng 2\n"});
        let details = write_file_details(&Ws(Content::Missing), &args).expect("phải có chi tiết");
        assert!(details.contains("tạo file mới (2 dòng)"), "{details}");
        assert!(details.contains("+dòng 1"), "{details}");
    }

    #[test]
    fn write_details_reports_overwrite_with_diff() {
        let args = json!({"path": "f.txt", "content": "cũ\nmới\n"});
        let details =
            write_file_details(&Ws(Content::Text("cũ\nđổi\n")), &args).expect("phải có chi tiết");
        assert!(details.contains("ghi đè (2 dòng → 2 dòng)"), "{details}");
        assert!(details.contains("-đổi"), "{details}");
        assert!(details.contains("+mới"), "{details}");
    }

    #[test]
    fn write_details_notes_binary_old_file() {
        let args = json!({"path": "img.bin", "content": "abc\n"});
        let details = write_file_details(&Ws(Content::Binary), &args).expect("phải có chi tiết");
        assert!(details.contains("nhị phân"), "{details}");
        assert!(
            !details.contains("\n--- a/"),
            "không có khối diff: {details}"
        );
    }

    #[test]
    fn write_details_returns_none_on_other_io_error() {
        let args = json!({"path": "f.txt", "content": "abc\n"});
        assert!(write_file_details(&Ws(Content::IoError), &args).is_none());
    }

    #[test]
    fn write_details_returns_none_without_args() {
        assert!(write_file_details(&Ws(Content::Text("x")), &json!({})).is_none());
        assert!(write_file_details(&Ws(Content::Text("x")), &json!({"path": 3})).is_none());
    }

    #[test]
    fn edit_details_shows_diff_for_unique_match() {
        let args = json!({"path": "src.rs", "old": "xin chào", "new": "tạm biệt"});
        let ws = Ws(Content::Text(
            "fn main() {\n    println!(\"xin chào\");\n}\n",
        ));
        let details = edit_file_details(&ws, &args).expect("phải có chi tiết");
        assert!(details.contains("thay đúng 1 chỗ"), "{details}");
        assert!(
            details.contains("-    println!(\"xin chào\");"),
            "{details}"
        );
        assert!(
            details.contains("+    println!(\"tạm biệt\");"),
            "{details}"
        );
    }

    #[test]
    fn edit_details_warns_when_old_not_unique() {
        let args = json!({"path": "f.txt", "old": "abc", "new": "x"});
        let ws = Ws(Content::Text("abc\nabc\n"));
        let details = edit_file_details(&ws, &args).expect("phải có chi tiết");
        assert!(details.contains("xuất hiện 2 lần"), "{details}");
        assert!(
            !details.contains("--- a/"),
            "không có diff khi sẽ lỗi: {details}"
        );
    }

    #[test]
    fn edit_details_returns_none_when_file_unreadable() {
        let args = json!({"path": "f.txt", "old": "a", "new": "b"});
        assert!(edit_file_details(&Ws(Content::Missing), &args).is_none());
    }

    /// Wire kiểm chứng end-to-end: hàm details gắn vào đúng tool thật qua builder.
    #[test]
    fn built_tools_expose_confirm_details() {
        let wf = write_file();
        let args = json!({"path": "moi.md", "content": "nội dung\n"});
        let ws = Ws(Content::Missing);
        let details = wf
            .confirm_details(&ws, &args)
            .expect("write_file phải có chi tiết");
        assert!(details.contains("tạo file mới"), "{details}");

        let ef = edit_file();
        let args = json!({"path": "f.txt", "old": "abc", "new": "x"});
        let ws = Ws(Content::Text("abc\n"));
        let details = ef
            .confirm_details(&ws, &args)
            .expect("edit_file phải có chi tiết");
        assert!(details.contains("thay đúng 1 chỗ"), "{details}");
    }
}
