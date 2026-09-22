//! Implement tool cho nhóm file (agents.md mục 7.3).

use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
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
    Arc::new(TypedTool::new(
        "read_file",
        Risk::Safe,
        |ctx: &ToolCtx, p: ReadFileParams| {
            let ws = ctx.workspace.clone();
            let path = p.path.clone();
            let offset = p.offset;
            let limit = p.limit;
            async move { read_capped(ws, &path, offset, limit).await }
        },
    ))
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
    let buf = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ToolError> {
        let path = ws2.resolve(&rel_for_closure)?;
        let meta = std::fs::metadata(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ToolError::NotFound(rel_for_closure.clone())
            } else {
                ToolError::Io(e.to_string())
            }
        })?;
        if !meta.is_file() {
            return Err(ToolError::Io(format!(
                "`{rel_for_closure}` không phải file"
            )));
        }
        let size = meta.len();
        if offset >= size {
            return Ok(Vec::new());
        }
        let mut file = std::fs::File::open(&path).map_err(|e| ToolError::Io(e.to_string()))?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| ToolError::Io(e.to_string()))?;
        }
        let mut v = Vec::with_capacity(n);
        std::io::Read::take(&mut file, n as u64)
            .read_to_end(&mut v)
            .map_err(|e| ToolError::Io(e.to_string()))?;
        Ok(v)
    })
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
    Arc::new(TypedTool::new(
        "list_dir",
        Risk::Safe,
        |ctx: &ToolCtx, p: ListDirParams| {
            let ws = ctx.workspace.clone();
            let path = p.path.clone();
            async move {
                let rel = if path.trim().is_empty() { "." } else { &path };
                let entries = ws.list_dir(rel)?;
                serde_json::to_string(&entries).map_err(|e| ToolError::Internal(e.to_string()))
            }
        },
    ))
}

/// Tool tìm file theo glob (Safe).
pub fn glob() -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
        "glob",
        Risk::Safe,
        |ctx: &ToolCtx, p: GlobParams| {
            let ws = ctx.workspace.clone();
            let pattern = p.pattern.clone();
            let limit = p.limit;
            async move {
                let paths = ws.glob(&pattern, limit)?;
                serde_json::to_string(&paths).map_err(|e| ToolError::Internal(e.to_string()))
            }
        },
    ))
}

/// Tool tìm regex (Safe).
pub fn grep() -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
        "grep",
        Risk::Safe,
        |ctx: &ToolCtx, p: GrepParams| {
            let ws = ctx.workspace.clone();
            let pattern = p.pattern.clone();
            let rel = p.rel.clone();
            let limit = p.limit;
            async move {
                let matches = ws.grep(&pattern, rel.as_deref(), limit)?;
                serde_json::to_string(&matches).map_err(|e| ToolError::Internal(e.to_string()))
            }
        },
    ))
}

/// Tool ghi file (Confirm).
pub fn write_file() -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
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
    ))
}

/// Tool sửa file (Confirm).
pub fn edit_file() -> Arc<dyn Tool> {
    Arc::new(TypedTool::new(
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
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::workspace::FsWorkspace;
    use tempfile::tempdir;

    #[tokio::test]
    async fn read_write_and_grep_roundtrip() {
        let dir = tempdir().unwrap();
        let ws = Arc::new(FsWorkspace::open(dir.path().to_path_buf()).unwrap());
        ws.write_text("hello.txt", "Xin chào 🦀\nDòng thứ hai ツ\n")
            .unwrap();

        let got = read_capped(ws.clone(), "hello.txt", 0, 100).await.unwrap();
        assert!(got.contains("Xin chào"));
        assert!(got.contains("🦀"));

        let matches = ws.grep("chào", None, 10).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "hello.txt");
    }

    #[tokio::test]
    async fn read_offset_seeks_correctly() {
        let dir = tempdir().unwrap();
        let ws = Arc::new(FsWorkspace::open(dir.path().to_path_buf()).unwrap());
        ws.write_text("t.txt", "ABCDEabcde").unwrap();
        let part = read_capped(ws.clone(), "t.txt", 3, 5).await.unwrap();
        assert_eq!(part, "DEabc");
    }

    #[tokio::test]
    async fn read_missing_file_is_not_found() {
        let dir = tempdir().unwrap();
        let ws = Arc::new(FsWorkspace::open(dir.path().to_path_buf()).unwrap());
        let err = read_capped(ws, "nope.txt", 0, 100).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }
}
