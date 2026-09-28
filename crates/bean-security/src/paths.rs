//! Path jail bằng `cap_std::fs::Dir` (agents.md mục 15.1).
//!
//! [`CapWorkspace`] cài trait `bean_tools::WorkspaceFs`: **mọi** thao tác file của
//! tool (`read_file`, `write_file`, `list_dir`, `glob`, `grep`, `edit_file`) đi qua
//! `cap_std::fs::Dir` gốc workspace. cap-std dùng `openat2(RESOLVE_BENEATH)` trên Linux
//! (fallback: đi từng component với `O_NOFOLLOW`), nên:
//!
//! * đường dẫn tuyệt đối (`/etc/passwd`) → lỗi;
//! * `..` thoát ra ngoài → lỗi;
//! * **symlink trong workspace trỏ ra ngoài → lỗi** (điểm yếu của cài đặt M3);
//! * symlink trỏ vào trong sandbox vẫn hoạt động bình thường.
//!
//! Quyền `ambient_authority` được cấp **một lần duy nhất** khi mở workspace theo cấu hình
//! của người dùng (`[agent] workspace`) — sau đó không còn đường nào chạm `std::fs` bằng
//! đường dẫn tự nối chuỗi.
//!
//! Đường dẫn đầu vào là **chuỗi tương đối**; kiểm tra thêm trước khi đưa vào `Dir` để
//! thông báo lỗi dễ hiểu hơn (cap-std vẫn là lớp thực thi cuối cùng).

use std::io;
use std::path::{Component, Path, PathBuf};

use bean_tools::{DirEntryInfo, GrepMatch, ToolError, WorkspaceFs};
use cap_std::fs::Dir;
use globset::{GlobBuilder, GlobMatcher};

/// Workspace bị jail trong capability `Dir`.
#[derive(Debug)]
pub struct CapWorkspace {
    dir: Dir,
    /// Đường dẫn gốc tuyệt đối (chỉ để hiển thị — KHÔNG dùng để mở file).
    root: PathBuf,
    /// Trần kích thước file khi đọc toàn bộ (`read_text`, `grep`).
    max_file_bytes: u64,
}

impl CapWorkspace {
    /// Mở workspace tại `root` (không tự tạo — caller tạo trước khi gọi, như M3).
    ///
    /// # Errors
    /// [`ToolError::Io`] khi `root` không tồn tại / không phải thư mục / không mở được.
    pub fn open(root: PathBuf) -> Result<Self, ToolError> {
        let meta = std::fs::metadata(&root).map_err(|e| {
            ToolError::Io(format!("không mở được workspace {}: {e}", root.display()))
        })?;
        if !meta.is_dir() {
            return Err(ToolError::Io(format!(
                "{} không phải thư mục",
                root.display()
            )));
        }
        // Bất biến: đây là điểm DUY NHẤT trong M4 cấp quyền ambient — theo cấu hình
        // người dùng (`[agent] workspace`), do người vận hành chọn, một lần lúc khởi động.
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(|e| {
            ToolError::Io(format!("không mở được workspace {}: {e}", root.display()))
        })?;
        Ok(Self {
            dir,
            root,
            max_file_bytes: 10 * 1024 * 1024, // 10 MiB
        })
    }

    /// Đặt trần kích thước file khi đọc toàn bộ.
    #[must_use]
    pub const fn with_max_file_bytes(mut self, max: u64) -> Self {
        self.max_file_bytes = max;
        self
    }

    /// Kiểm tra đường dẫn tương đối an toàn: không rỗng, không NUL, không tuyệt đối,
    /// không có thành phần `..`. Trả về đường dẫn đã chuẩn hoá (bỏ `./`, giữ `.` cho root).
    fn check_rel(rel: &str) -> Result<PathBuf, ToolError> {
        if rel.is_empty() {
            return Ok(PathBuf::from("."));
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
            normalized.push(".");
        }
        Ok(normalized)
    }

    /// Duyệt đệ quy cây (bắt đầu từ thư mục con `rel`, `.` = root), trả các đường dẫn
    /// tương đối của **file** (bỏ symlink, bỏ thư mục), sort tăng dần.
    fn walk_files(&self, rel: &Path) -> Result<Vec<String>, ToolError> {
        let mut out = Vec::new();
        let start_dir = self.dir.open_dir(rel).map_err(|e| self.map_io(rel, e))?;
        let mut prefix = rel.to_path_buf();
        if prefix.as_os_str() == "." {
            prefix.clear();
        }
        walk_dir_recursive(&start_dir, &prefix, 0, &mut out);
        out.sort();
        Ok(out)
    }

    fn map_io(&self, rel: &Path, err: io::Error) -> ToolError {
        let rel_str = rel.to_string_lossy();
        if err.kind() == io::ErrorKind::NotFound {
            ToolError::NotFound(rel_str.to_string())
        } else {
            ToolError::Io(format!("`{rel_str}`: {err}"))
        }
    }
}

/// Trần duyệt đệ quy cho `glob`/`grep`.
const MAX_WALK_DEPTH: usize = 32;

/// Duyệt đệ quy từ một `Dir` capability — không bao giờ rời khỏi sandbox.
///
/// Symlink bị bỏ qua hoàn toàn (không đi vào symlink-dir, không đọc symlink-file)
/// để output `glob`/`grep` deterministic và tránh vòng lặp.
fn walk_dir_recursive(dir: &Dir, prefix: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    let entries = match dir.entries() {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!("bỏ qua thư mục không đọc được ({prefix:?}): {err}");
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                tracing::debug!("bỏ qua mục không đọc được: {err}");
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        let child_rel = prefix.join(entry.file_name());
        if file_type.is_dir() {
            // Mở thư mục con qua capability (không bao giờ rời sandbox) rồi đệ quy.
            if let Ok(sub) = entry.open_dir() {
                walk_dir_recursive(&sub, &child_rel, depth + 1, out);
            }
        } else if file_type.is_file() {
            out.push(child_rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

impl WorkspaceFs for CapWorkspace {
    fn root(&self) -> &Path {
        &self.root
    }

    /// Chuẩn hoá đường dẫn tương đối — **chỉ để hiển thị/log**; mọi đọc/ghi thực sự
    /// chạy qua `self.dir` nên kể cả khi "resolve" được đường dẫn cũng không mở được
    /// file bên ngoài (cap-std enforce theo capability).
    fn resolve(&self, rel: &str) -> Result<PathBuf, ToolError> {
        Self::check_rel(rel)
    }

    fn read_range(&self, rel: &str, offset: u64, limit: u64) -> Result<Vec<u8>, ToolError> {
        let path = Self::check_rel(rel)?;
        let meta = self
            .dir
            .metadata(&path)
            .map_err(|e| self.map_io(&path, e))?;
        if !meta.is_file() {
            return Err(ToolError::Io(format!("`{rel}` không phải file")));
        }
        if offset >= meta.len() {
            return Ok(Vec::new());
        }
        let mut file = self.dir.open(&path).map_err(|e| self.map_io(&path, e))?;
        use std::io::{Read, Seek, SeekFrom};
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| ToolError::Io(format!("`{rel}`: {e}")))?;
        }
        let mut buf = Vec::with_capacity(limit.min(64 * 1024) as usize);
        file.take(limit)
            .read_to_end(&mut buf)
            .map_err(|e| ToolError::Io(format!("`{rel}`: {e}")))?;
        Ok(buf)
    }

    fn read_text(&self, rel: &str) -> Result<String, ToolError> {
        let path = Self::check_rel(rel)?;
        let meta = self
            .dir
            .metadata(&path)
            .map_err(|e| self.map_io(&path, e))?;
        if meta.len() > self.max_file_bytes {
            return Err(ToolError::Io(format!(
                "`{rel}` quá lớn ({} byte > trần {} byte) — không đọc trực tiếp",
                meta.len(),
                self.max_file_bytes
            )));
        }
        let bytes = self.dir.read(&path).map_err(|e| self.map_io(&path, e))?;
        String::from_utf8(bytes).map_err(|_| ToolError::InvalidData(rel.to_string()))
    }

    fn write_text(&self, rel: &str, content: &str) -> Result<(), ToolError> {
        let path = Self::check_rel(rel)?;
        if let Some(parent) = path.parent()
            && parent.as_os_str() != "."
            && !parent.as_os_str().is_empty()
        {
            self.dir
                .create_dir_all(parent)
                .map_err(|e| self.map_io(parent, e))?;
        }
        self.dir
            .write(&path, content.as_bytes())
            .map_err(|e| self.map_io(&path, e))
    }

    fn edit_unique(&self, rel: &str, old: &str, new: &str) -> Result<(), ToolError> {
        let content = self.read_text(rel)?;
        let occurrences = content.matches(old).count();
        if occurrences == 0 {
            return Err(ToolError::NotFound(format!(
                "không tìm thấy chuỗi `{old}` trong `{rel}`"
            )));
        }
        if occurrences > 1 {
            return Err(ToolError::InvalidArgs(format!(
                "chuỗi `{old}` xuất hiện {occurrences} lần trong `{rel}` — phải cung cấp chuỗi đủ dài để duy nhất"
            )));
        }
        self.write_text(rel, &content.replacen(old, new, 1))
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<DirEntryInfo>, ToolError> {
        let path = Self::check_rel(rel)?;
        let entries = self
            .dir
            .read_dir(&path)
            .map_err(|e| self.map_io(&path, e))?;
        let mut out = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    tracing::debug!("bỏ qua mục không đọc được trong `{rel}`: {err}");
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().to_string();
            let file_type = entry.file_type().ok();
            // metadata() đi qua capability — symlink trỏ ra ngoài => lỗi => size 0.
            let meta = self.dir.metadata(path.join(entry.file_name())).ok();
            let meta_is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            let is_dir = file_type.is_some_and(|ft| ft.is_dir()) || meta_is_dir;
            let size = if is_dir {
                0
            } else {
                meta.as_ref().map_or(0, |m| m.len())
            };
            out.push(DirEntryInfo { name, is_dir, size });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn glob(&self, pattern: &str, limit: usize) -> Result<Vec<String>, ToolError> {
        if pattern.trim().is_empty() {
            return Ok(Vec::new());
        }
        let matcher = compile_glob(pattern)?;
        let files = self.walk_files(Path::new("."))?;
        Ok(files
            .into_iter()
            .filter(|rel| matcher.is_match(rel))
            .take(limit)
            .collect())
    }

    fn grep(
        &self,
        pattern: &str,
        rel: Option<&str>,
        limit: usize,
    ) -> Result<Vec<GrepMatch>, ToolError> {
        let re = bean_tools::compile_regex(pattern)?;
        let scan_root = match rel {
            None | Some("") | Some(".") => PathBuf::from("."),
            Some(r) => Self::check_rel(r)?,
        };
        let is_file = self
            .dir
            .metadata(&scan_root)
            .map_err(|e| self.map_io(&scan_root, e))?
            .is_file();
        let files = if is_file {
            vec![scan_root.to_string_lossy().replace('\\', "/")]
        } else {
            self.walk_files(&scan_root)?
        };

        let mut out = Vec::new();
        for file_rel in files {
            if out.len() >= limit {
                break;
            }
            // Đọc có trần; file quá lớn/lỗi đọc bị bỏ qua — grep không được sập (mục 6).
            let bytes = match self.dir.read(&file_rel) {
                Ok(bytes) if bytes.len() as u64 <= self.max_file_bytes => bytes,
                Ok(_) => {
                    tracing::debug!("grep bỏ qua file quá lớn: {file_rel}");
                    continue;
                }
                Err(err) => {
                    tracing::debug!("grep bỏ qua file lỗi `{file_rel}`: {err}");
                    continue;
                }
            };
            // Bỏ file nhị phân (NUL trong 8000 byte đầu) — như M3.
            if bytes[..bytes.len().min(8000)].contains(&0u8) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (idx, line) in text.lines().enumerate() {
                if out.len() >= limit {
                    return Ok(out);
                }
                if re.is_match(line) {
                    let (shown, was_cut) = bean_tools::truncate_chars(line, 200)
                        .map_or((line, false), |(kept, _)| (kept, true));
                    let suffix = if was_cut { "…" } else { "" };
                    out.push(GrepMatch {
                        path: file_rel.clone(),
                        line_no: idx + 1,
                        line: format!("{shown}{suffix}"),
                    });
                }
            }
        }
        Ok(out)
    }
}

fn compile_glob(pattern: &str) -> Result<GlobMatcher, ToolError> {
    GlobBuilder::new(pattern)
        .literal_separator(true) // `*` không vượt ranh giới `/` (như M3)
        .build()
        .map(|g| g.compile_matcher())
        .map_err(|e| ToolError::InvalidArgs(format!("glob không hợp lệ `{pattern}`: {e}")))
}
