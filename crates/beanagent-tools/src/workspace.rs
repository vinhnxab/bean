//! Truy cập file bị jail trong workspace (agents.md mục 15.1, 7.1).
//!
//! M3 cài [`FsWorkspace`] với kiểm tra đơn giản: cấm đường dẫn tuyệt đối và các thành
//! phần `..`; đường dẫn được chuẩn hoá rồi nối vào thư mục gốc workspace.
//! **M4** thay bằng `cap_std::fs::Dir` (capability thật, chặn cả symlink thoát ra) — chỉ cần
//! cài lại trait này, mọi tool builtin không phải sửa.
//!
//! Mọi hàm là **blocking** (std::fs); caller async phải bọc `spawn_blocking`.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

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

/// Capability truy cập file trong workspace — seam để M4 thay bằng cap-std.
pub trait WorkspaceFs: Send + Sync + std::fmt::Debug {
    /// Đường dẫn gốc tuyệt đối (để hiển thị, không dùng để đọc file trực tiếp).
    fn root(&self) -> &Path;

    /// Kiểm tra và chuẩn hoá đường dẫn tương đối; lỗi [`ToolError::NotInWorkspace`]
    /// khi tuyệt đối, chứa `..`, hoặc rỗng.
    ///
    /// Lưu ý M3: **chưa** resolve symlink (làm ở M4 với cap-std).
    fn resolve(&self, rel: &str) -> Result<PathBuf, ToolError>;

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

/// Cài đặt M3 của [`WorkspaceFs`] trên `std::fs`.
#[derive(Debug)]
pub struct FsWorkspace {
    root: PathBuf,
    /// Trần kích thước file khi đọc văn bản (chống đọc file khổng lồ vào bộ nhớ).
    max_file_bytes: u64,
}

impl FsWorkspace {
    /// Mở workspace tại `root` (không tự tạo — caller chịu trách nhiệm tạo thư mục).
    ///
    /// # Errors
    /// [`ToolError::Io`] khi `root` không phải thư mục hoặc không truy cập được.
    pub fn open(root: PathBuf) -> Result<Self, ToolError> {
        let meta =
            fs::metadata(&root).map_err(|e| ToolError::Io(format!("{}: {e}", root.display())))?;
        if !meta.is_dir() {
            return Err(ToolError::Io(format!(
                "{} không phải thư mục",
                root.display()
            )));
        }
        Ok(Self {
            root,
            max_file_bytes: 10 * 1024 * 1024, // 10 MiB
        })
    }

    /// Đặt trần kích thước file khi đọc văn bản.
    #[must_use]
    pub const fn with_max_file_bytes(mut self, max: u64) -> Self {
        self.max_file_bytes = max;
        self
    }

    /// Kiểm tra đường dẫn tương đối an toàn ở mức M3: không tuyệt đối, không `..`,
    /// không chứa NUL; trả về đường dẫn đã chuẩn hoá (bỏ `./`).
    fn check_rel(rel: &str) -> Result<PathBuf, ToolError> {
        if rel.is_empty() {
            return Err(ToolError::NotInWorkspace("đường dẫn rỗng".to_string()));
        }
        if rel.contains('\0') {
            return Err(ToolError::NotInWorkspace(
                "đường dẫn chứa ký tự NUL".to_string(),
            ));
        }
        let path = Path::new(rel);
        if path.is_absolute() {
            return Err(ToolError::NotInWorkspace(format!(
                "`{rel}` là đường dẫn tuyệt đối — chỉ dùng đường dẫn tương đối với workspace"
            )));
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(part) => normalized.push(part),
                Component::ParentDir => {
                    return Err(ToolError::NotInWorkspace(format!(
                        "`{rel}` chứa `..` — không được phép thoát khỏi workspace"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(ToolError::NotInWorkspace(format!(
                        "`{rel}` không phải đường dẫn tương đối hợp lệ"
                    )));
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(ToolError::NotInWorkspace(format!(
                "`{rel}` không trỏ tới file/thư mục nào"
            )));
        }
        Ok(normalized)
    }

    fn physical(&self, rel: &str) -> Result<PathBuf, ToolError> {
        Ok(self.root.join(Self::check_rel(rel)?))
    }

    /// Duyệt toàn bộ cây, trả các đường dẫn tương đối của file/thư mục.
    /// Bỏ qua các mục không đọc được (log debug) — thư mục lỗi không được làm
    /// `glob`/`grep` sập (mục 6).
    fn walk_rel(&self) -> Vec<PathBuf> {
        walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::debug!("bỏ qua mục không đọc được khi duyệt: {err}");
                        return None;
                    }
                };
                if entry.depth() == 0 {
                    return None; // bỏ chính workspace root
                }
                let rel = entry.path().strip_prefix(&self.root).ok()?;
                if rel
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return None;
                }
                Some(rel.to_path_buf())
            })
            .collect()
    }

    fn read_capped(&self, path: &Path, rel: &str) -> Result<Vec<u8>, ToolError> {
        let meta = fs::metadata(path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ToolError::NotFound(rel.to_string())
            } else {
                ToolError::Io(format!("`{rel}`: {e}"))
            }
        })?;
        if meta.len() > self.max_file_bytes {
            return Err(ToolError::Io(format!(
                "`{rel}` quá lớn ({} byte > trần {} byte) — không đọc trực tiếp",
                meta.len(),
                self.max_file_bytes
            )));
        }
        fs::read(path).map_err(|e| ToolError::Io(format!("`{rel}`: {e}")))
    }

    /// Danh sách `(đường_dẫn_tuyệt_đối, đường_dẫn_hiển thị)` để `grep` quét.
    ///
    /// `rel = None` ⇒ toàn workspace (duyệt cây); `rel` trỏ tới file ⇒ một mục; trỏ tới
    /// thư mục ⇒ duyệt cây con đó.
    fn scan_targets(&self, rel: Option<&str>) -> Result<Vec<(PathBuf, String)>, ToolError> {
        let Some(rel) = rel else {
            return Ok(self
                .walk_rel()
                .into_iter()
                .filter(|p| self.root.join(p).is_file())
                .map(|p| {
                    let display = p.to_string_lossy().replace('\\', "/");
                    (self.root.join(&p), display)
                })
                .collect());
        };
        let physical = self.physical(rel)?;
        let meta = fs::metadata(&physical).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ToolError::NotFound(rel.to_string())
            } else {
                ToolError::Io(format!("`{rel}`: {e}"))
            }
        })?;
        if meta.is_file() {
            return Ok(vec![(physical, rel.replace('\\', "/"))]);
        }
        // Thư mục: duyệt cây con, đường dẫn hiển thị = rel/thành_phần.
        let base = PathBuf::from(rel.replace('\\', "/"));
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&physical)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| match e {
                Ok(e) => Some(e),
                Err(err) => {
                    tracing::debug!("bỏ qua mục không đọc được khi duyệt: {err}");
                    None
                }
            })
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let sub = entry
                .path()
                .strip_prefix(&physical)
                .map_err(|e| ToolError::Internal(e.to_string()))?;
            let display = if sub.as_os_str().is_empty() {
                base.to_string_lossy().replace('\\', "/")
            } else {
                base.join(sub).to_string_lossy().replace('\\', "/")
            };
            out.push((entry.path().to_path_buf(), display));
        }
        out.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(out)
    }
}

impl WorkspaceFs for FsWorkspace {
    fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, rel: &str) -> Result<PathBuf, ToolError> {
        Ok(self.root.join(Self::check_rel(rel)?))
    }

    fn read_text(&self, rel: &str) -> Result<String, ToolError> {
        let path = self.physical(rel)?;
        let bytes = self.read_capped(&path, rel)?;
        String::from_utf8(bytes).map_err(|_| ToolError::InvalidData(rel.to_string()))
    }

    fn write_text(&self, rel: &str, content: &str) -> Result<(), ToolError> {
        let path = self.physical(rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ToolError::Io(format!("`{rel}`: {e}")))?;
        }
        fs::write(&path, content).map_err(|e| ToolError::Io(format!("`{rel}`: {e}")))
    }

    fn edit_unique(&self, rel: &str, old: &str, new: &str) -> Result<(), ToolError> {
        if old.is_empty() {
            return Err(ToolError::InvalidArgs(
                "`old` rỗng — phải là chuỗi có nội dung".to_string(),
            ));
        }
        let content = self.read_text(rel)?;
        let count = content.matches(old).count();
        match count {
            0 => Err(ToolError::NotFound(format!(
                "`{rel}`: không tìm thấy chuỗi cần thay"
            ))),
            1 => {
                let updated = content.replacen(old, new, 1);
                self.write_text(rel, &updated)
            }
            _ => Err(ToolError::InvalidArgs(format!(
                "`{rel}`: chuỗi cần thay xuất hiện {count} lần — hãy đưa chuỗi dài hơn để duy nhất"
            ))),
        }
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirEntryInfo>, ToolError> {
        let path = self.physical(rel)?;
        let meta = fs::metadata(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                ToolError::NotFound(rel.to_string())
            } else {
                ToolError::Io(format!("`{rel}`: {e}"))
            }
        })?;
        if !meta.is_dir() {
            return Err(ToolError::Io(format!("`{rel}` không phải thư mục")));
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&path).map_err(|e| ToolError::Io(format!("`{rel}`: {e}")))? {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!("`{rel}`: bỏ qua mục không đọc được: {e}");
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(e) => {
                    tracing::debug!("`{rel}`: bỏ qua mục không xác định kiểu: {e}");
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(DirEntryInfo {
                name,
                is_dir: file_type.is_dir(),
                size,
            });
        }
        // Thư mục trước, rồi tên tăng dần.
        out.sort_by(|a, b| (&b.is_dir, &a.name).cmp(&(&a.is_dir, &b.name)));
        Ok(out)
    }

    fn glob(&self, pattern: &str, limit: usize) -> Result<Vec<String>, ToolError> {
        if pattern.trim().is_empty() {
            return Ok(Vec::new());
        }
        let g = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|e| ToolError::InvalidArgs(format!("glob sai cú pháp: {e}")))?;
        let gs = globset::GlobSetBuilder::new()
            .add(g)
            .build()
            .map_err(|e| ToolError::InvalidArgs(format!("glob sai cú pháp: {e}")))?;
        let mut out = Vec::new();
        for rel in self.walk_rel() {
            let text = rel.to_string_lossy().replace('\\', "/");
            if gs.is_match(&text) && out.len() < limit {
                out.push(text);
            }
        }
        out.sort();
        Ok(out)
    }

    fn grep(
        &self,
        pattern: &str,
        rel: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GrepMatch>, ToolError> {
        let re = regex::Regex::new(pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("regex sai cú pháp: {e}")))?;
        let targets = self.scan_targets(rel)?;
        let mut out = Vec::new();
        for (path, shown_path) in targets {
            if out.len() >= limit {
                break;
            }
            let bytes = match self.read_capped(&path, &shown_path) {
                Ok(b) => b,
                Err(err) => {
                    tracing::debug!("grep bo qua {}: {:?}", shown_path, err);
                    continue;
                }
            };
            let Ok(content) = std::str::from_utf8(&bytes) else {
                continue; // file nhị phân — bỏ im lặng
            };
            for (idx, line) in content.lines().enumerate() {
                if out.len() >= limit {
                    return Ok(out);
                }
                if re.is_match(line) {
                    let (shown, was_cut) = crate::text::truncate_chars(line, 200)
                        .map_or((line, false), |(kept, _)| (kept, true));
                    let suffix = if was_cut { "…" } else { "" };
                    out.push(GrepMatch {
                        path: shown_path.clone(),
                        line_no: idx + 1,
                        line: format!("{shown}{suffix}"),
                    });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn ws() -> (tempfile::TempDir, FsWorkspace) {
        let dir = tempfile::tempdir().unwrap();
        let ws = FsWorkspace::open(dir.path().to_path_buf()).unwrap();
        (dir, ws)
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        let (_dir, ws) = ws();
        assert!(ws.resolve("/etc/passwd").is_err());
        assert!(ws.resolve("../outside.txt").is_err());
        assert!(ws.resolve("a/../../b").is_err());
        assert!(ws.resolve("").is_err());
        assert!(ws.resolve("\0bad").is_err());
        assert!(ws.resolve("ok/child.txt").is_ok());
        assert!(ws.resolve("./ok.txt").is_ok());
    }

    #[test]
    fn write_read_edit_roundtrip() {
        let (_dir, ws) = ws();
        ws.write_text("sub/a.txt", "hello\n").unwrap();
        assert_eq!(ws.read_text("sub/a.txt").unwrap(), "hello\n");
        ws.edit_unique("sub/a.txt", "hello", "xin chào 🦀").unwrap();
        assert_eq!(ws.read_text("sub/a.txt").unwrap(), "xin chào 🦀\n");
    }

    #[test]
    fn edit_requires_unique_match() {
        let (_dir, ws) = ws();
        ws.write_text("f.txt", "ab ab").unwrap();
        assert!(ws.edit_unique("f.txt", "ab", "x").is_err());
        assert!(ws.edit_unique("f.txt", "ab ab", "x").is_ok());
        assert_eq!(ws.read_text("f.txt").unwrap(), "x");
    }

    #[test]
    fn glob_and_grep_walk_tree() {
        let (_dir, ws) = ws();
        ws.write_text("src/main.rs", "fn main() {}\n").unwrap();
        ws.write_text("src/lib/deep.rs", "const HÈ: &str = \"việt\";\n")
            .unwrap();
        ws.write_text("notes.md", "tìm em 🦀\n").unwrap();

        let rs = ws.glob("**/*.rs", 50).unwrap();
        assert_eq!(rs, vec!["src/lib/deep.rs", "src/main.rs"]);
        // `*` không vượt qua ranh giới thư mục:
        assert!(ws.glob("*.rs", 50).unwrap().is_empty());

        let hits = ws.grep("việt", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/lib/deep.rs");
        assert_eq!(hits[0].line_no, 1);
    }

    #[test]
    fn grep_skips_binary_files() {
        let (_dir, ws) = ws();
        std::fs::write(ws.root().join("blob.bin"), [0xFF_u8, 0xFE, 0x00, 0x01]).unwrap();
        ws.write_text("t.txt", "hello\n").unwrap();
        let hits = ws.grep("hello", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "t.txt");
    }

    #[test]
    fn read_reports_missing_file_not_found() {
        let (_dir, ws) = ws();
        let err = ws.read_text("nope.txt").unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)), "{err:?}");
    }
}
