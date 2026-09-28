//! Loader skill: quét thư mục, parse frontmatter tối giển và quản lý catalog.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use thiserror::Error;
use tracing::{debug, warn};

use crate::drafts::DraftRuntime;

/// Tên file bắt buộc trong mỗi thư mục skill.
pub const SKILL_FILE: &str = "SKILL.md";
/// Trần độ dài description theo số ký tự Unicode.
pub const MAX_DESCRIPTION_CHARS: usize = 300;
/// Trần an toàn cho SKILL.md do reflection sinh.
pub const MAX_SKILL_CHARS: usize = 100_000;

/// Một skill đã được loader kiểm tra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Tên kebab-case, cũng là tên thư mục.
    pub name: String,
    /// Mô tả ngắn để đưa vào system prompt.
    pub description: String,
    /// Đường dẫn tuyệt đối tới thư mục skill.
    pub directory: PathBuf,
    /// Toàn bộ nội dung `SKILL.md`, chỉ được trả về qua `load_skill`.
    pub content: String,
}

/// Lỗi khi đọc, kiểm tra hoặc tạo skill.
#[derive(Debug, Error)]
pub enum SkillError {
    /// Frontmatter không đúng hoặc có field không hợp lệ.
    #[error("frontmatter không hợp lệ: {0}")]
    InvalidFrontmatter(String),
    /// Thiếu `name` hoặc `description`.
    #[error("thiếu trường frontmatter `{0}`")]
    MissingField(&'static str),
    /// Frontmatter hợp lệ nhưng phần hướng dẫn bên dưới rỗng.
    #[error("nội dung hướng dẫn của skill rỗng")]
    MissingBody,
    /// Tên không phải kebab-case hoặc chứa ký tự đường dẫn.
    #[error("tên skill không hợp lệ: {0}")]
    InvalidName(String),
    /// Description rỗng hoặc vượt trần.
    #[error("description không hợp lệ: {0}")]
    InvalidDescription(String),
    /// Skill đã tồn tại; không được ghi đè.
    #[error("skill đã tồn tại: {0}")]
    AlreadyExists(String),
    /// Đã có một draft pending cho cùng tên skill.
    #[error("skill `{0}` đã có đề xuất đang chờ duyệt")]
    DraftAlreadyPending(String),
    /// Không tìm thấy draft theo ID.
    #[error("không tìm thấy skill nháp: {0}")]
    DraftNotFound(String),
    /// Skill nền đã thay đổi sau khi reflection tạo draft.
    #[error("skill `{0}` đã thay đổi sau khi đề xuất; hãy tạo lại draft")]
    DraftConflict(String),
    /// Metadata hoặc nội dung draft không hợp lệ.
    #[error("skill nháp không hợp lệ: {0}")]
    InvalidDraft(String),
    /// OS không cấp entropy để tạo ID draft.
    #[error("không sinh được ID skill nháp: {0}")]
    DraftRandom(String),
    /// Lỗi I/O.
    #[error("lỗi I/O skill: {0}")]
    Io(String),
    /// Không tìm thấy skill.
    #[error("không tìm thấy skill: {0}")]
    NotFound(String),
    /// RwLock của catalog bị poison.
    #[error("catalog skill không còn khả dụng")]
    CatalogPoisoned,
}

/// Catalog dùng chung cho system prompt và các tool.
#[derive(Clone)]
pub struct SkillCatalog {
    pub(crate) skills: Arc<RwLock<BTreeMap<String, Skill>>>,
    pub(crate) roots: Arc<Vec<PathBuf>>,
    pub(crate) create_root: PathBuf,
    pub(crate) draft_root: PathBuf,
    pub(crate) drafts: DraftRuntime,
}

impl std::fmt::Debug for SkillCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self
            .skills
            .read()
            .map(|skills| skills.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        f.debug_struct("SkillCatalog")
            .field("skills", &names)
            .field("create_root", &self.create_root)
            .field("draft_root", &self.draft_root)
            .finish()
    }
}

impl SkillCatalog {
    /// Quét các root theo thứ tự; root sau ghi đè root trùng tên.
    #[must_use]
    pub fn load(roots: &[PathBuf]) -> Self {
        let create_root = roots
            .last()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("skills"));
        Self::load_with_create_root(roots, create_root)
    }

    /// Như [`SkillCatalog::load`], nhưng chỉ định rõ thư mục dùng cho `create_skill`.
    #[must_use]
    pub fn load_with_create_root(roots: &[PathBuf], create_root: PathBuf) -> Self {
        let primary_root = roots
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("skills"));
        Self::load_with_paths(roots, create_root, primary_root.join("_drafts"))
    }

    /// Dựng catalog với đầy đủ các root, gồm thư mục chứa draft M15.
    #[must_use]
    pub fn load_with_paths(roots: &[PathBuf], create_root: PathBuf, draft_root: PathBuf) -> Self {
        let catalog = Self {
            skills: Arc::new(RwLock::new(BTreeMap::new())),
            roots: Arc::new(roots.to_vec()),
            create_root,
            draft_root: draft_root.clone(),
            drafts: DraftRuntime::load(&draft_root),
        };
        for root in roots {
            catalog.load_root(root);
        }
        catalog
    }

    /// Tạo index `name: description`, không chứa nội dung đầy đủ.
    #[must_use]
    pub fn index(&self) -> String {
        let skills = match self.skills.read() {
            Ok(skills) => skills,
            Err(_) => return String::new(),
        };
        skills
            .values()
            .map(|skill| format!("{}: {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Liệt kê snapshot skill để REST/API và UI dùng cùng một catalog.
    #[must_use]
    pub fn list(&self) -> Vec<Skill> {
        self.skills
            .read()
            .map(|skills| skills.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Lấy một skill theo tên.
    pub fn get(&self, name: &str) -> Result<Skill, SkillError> {
        validate_name(name)?;
        self.skills
            .read()
            .map_err(|_| SkillError::CatalogPoisoned)?
            .get(name)
            .cloned()
            .ok_or_else(|| SkillError::NotFound(name.to_string()))
    }

    /// Tạo skill mới trong root dành cho người dùng; tuyệt đối không ghi đè.
    pub fn create(&self, name: &str, description: &str, body: &str) -> Result<Skill, SkillError> {
        validate_name(name)?;
        let description = normalize_description(description)?;
        let content = render_skill(name, &description, body)?;
        {
            let skills = self
                .skills
                .read()
                .map_err(|_| SkillError::CatalogPoisoned)?;
            if skills.contains_key(name) {
                return Err(SkillError::AlreadyExists(name.to_string()));
            }
        }
        fs::create_dir_all(&self.create_root).map_err(|e| SkillError::Io(e.to_string()))?;
        let directory = self.create_root.join(name);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(SkillError::AlreadyExists(name.to_string()));
            }
            Err(e) => return Err(SkillError::Io(e.to_string())),
        }
        let file_path = directory.join(SKILL_FILE);
        let result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .and_then(|mut file| file.write_all(content.as_bytes()));
        if let Err(e) = result {
            let _ = fs::remove_file(&file_path);
            let _ = fs::remove_dir(&directory);
            return Err(SkillError::Io(e.to_string()));
        }
        let directory = fs::canonicalize(&directory).unwrap_or(directory);
        let skill = Skill {
            name: name.to_string(),
            description,
            directory: directory.clone(),
            content,
        };
        self.skills
            .write()
            .map_err(|_| SkillError::CatalogPoisoned)?
            .insert(name.to_string(), skill.clone());
        Ok(skill)
    }

    /// Nạp lại toàn bộ skill đang hoạt động từ các root.
    pub fn reload(&self) -> Result<(), SkillError> {
        let mut loaded = BTreeMap::new();
        for root in self.roots.iter() {
            if let Ok(entries) = fs::read_dir(root) {
                let mut directories = Vec::new();
                for entry in entries.flatten() {
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    if file_type.is_dir() && entry.file_name() != std::ffi::OsStr::new("_drafts") {
                        directories.push(entry.path());
                    }
                }
                directories.sort();
                for directory in directories {
                    match Self::read_skill(&directory) {
                        Ok(skill) => {
                            loaded.insert(skill.name.clone(), skill);
                        }
                        Err(error) => warn!(
                            path = %directory.display(),
                            error = %error,
                            "bỏ qua skill không hợp lệ khi reload"
                        ),
                    }
                }
            }
        }
        *self
            .skills
            .write()
            .map_err(|_| SkillError::CatalogPoisoned)? = loaded;
        Ok(())
    }

    fn load_root(&self, root: &Path) {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(root = %root.display(), "thư mục skill không tồn tại (bỏ qua)");
                return;
            }
            Err(e) => {
                warn!(root = %root.display(), error = %e, "không đọc được thư mục skill");
                return;
            }
        };
        let mut directories = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => match entry.file_type() {
                    Ok(file_type)
                        if file_type.is_dir()
                            && entry.file_name() != std::ffi::OsStr::new("_drafts") =>
                    {
                        directories.push(entry.path());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(path = %entry.path().display(), error = %e, "bỏ qua entry skill lỗi")
                    }
                },
                Err(e) => warn!(root = %root.display(), error = %e, "bỏ qua entry skill lỗi"),
            }
        }
        directories.sort();
        for directory in directories {
            match Self::read_skill(&directory) {
                Ok(skill) => self.insert_loaded(skill),
                Err(e) => {
                    warn!(path = %directory.display(), error = %e, "bỏ qua skill không hợp lệ")
                }
            }
        }
    }

    fn insert_loaded(&self, skill: Skill) {
        match self.skills.write() {
            Ok(mut skills) => {
                if let Some(previous) = skills.insert(skill.name.clone(), skill.clone()) {
                    warn!(name = %skill.name, previous = %previous.directory.display(), path = %skill.directory.display(), "skill trùng tên; ưu tiên root sau");
                }
            }
            Err(_) => warn!("không cập nhật catalog skill vì lock bị poison"),
        }
    }

    pub(crate) fn read_skill(directory: &Path) -> Result<Skill, SkillError> {
        let directory_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SkillError::InvalidName("tên thư mục không phải UTF-8".to_string()))?;
        let file_path = directory.join(SKILL_FILE);
        let metadata =
            fs::symlink_metadata(&file_path).map_err(|e| SkillError::Io(e.to_string()))?;
        if !metadata.file_type().is_file() {
            return Err(SkillError::InvalidFrontmatter(format!(
                "{SKILL_FILE} không phải file thường"
            )));
        }
        let content = fs::read_to_string(&file_path).map_err(|e| SkillError::Io(e.to_string()))?;
        validate_skill_size(&content)?;
        let (name, description) = parse_frontmatter(&content)?;
        if name != directory_name {
            return Err(SkillError::InvalidName(format!(
                "frontmatter `{name}` không khớp thư mục `{directory_name}`"
            )));
        }
        let directory = fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
        Ok(Skill {
            name,
            description,
            directory,
            content,
        })
    }
}

/// Parse đúng hai field scalar `name` và `description` trong frontmatter.
fn parse_frontmatter(content: &str) -> Result<(String, String), SkillError> {
    let lines: Vec<&str> = content.lines().collect();
    let mut lines = lines.iter().copied();
    if lines.next().map(str::trim) != Some("---") {
        return Err(SkillError::InvalidFrontmatter(
            "phải bắt đầu bằng `---`".to_string(),
        ));
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines.by_ref() {
        let line = line.trim();
        if line == "---" {
            closed = true;
            break;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, raw_value) = line
            .split_once(':')
            .ok_or_else(|| SkillError::InvalidFrontmatter(format!("dòng `{line}` thiếu `:`")))?;
        let key = key.trim();
        let value = parse_scalar(raw_value)?;
        match key {
            "name" if name.is_none() => name = Some(value),
            "description" if description.is_none() => description = Some(value),
            "name" | "description" => {
                return Err(SkillError::InvalidFrontmatter(format!(
                    "trường `{key}` bị lặp"
                )));
            }
            _ => {
                return Err(SkillError::InvalidFrontmatter(format!(
                    "field không được phép `{key}`"
                )));
            }
        }
    }
    if !closed {
        return Err(SkillError::InvalidFrontmatter(
            "thiếu dấu đóng `---`".to_string(),
        ));
    }
    if !lines.any(|line| !line.trim().is_empty()) {
        return Err(SkillError::MissingBody);
    }
    let name = name.ok_or(SkillError::MissingField("name"))?;
    let description = description.ok_or(SkillError::MissingField("description"))?;
    validate_name(&name)?;
    let description = normalize_description(&description)?;
    Ok((name, description))
}

pub(crate) fn render_skill(
    name: &str,
    description: &str,
    body: &str,
) -> Result<String, SkillError> {
    let description = normalize_description(description)?;
    if body.trim().is_empty() {
        return Err(SkillError::MissingBody);
    }
    let description_yaml =
        serde_json::to_string(&description).map_err(|error| SkillError::Io(error.to_string()))?;
    let content = format!("---\nname: {name}\ndescription: {description_yaml}\n---\n\n{body}\n");
    validate_skill_size(&content)?;
    Ok(content)
}

pub(crate) fn validate_skill_size(content: &str) -> Result<(), SkillError> {
    let length = content.chars().count();
    if length > MAX_SKILL_CHARS {
        return Err(SkillError::InvalidFrontmatter(format!(
            "SKILL.md dài {length} ký tự, tối đa {MAX_SKILL_CHARS}"
        )));
    }
    Ok(())
}

fn parse_scalar(raw: &str) -> Result<String, SkillError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(SkillError::InvalidFrontmatter("giá trị rỗng".to_string()));
    }
    if let Ok(value) = serde_json::from_str::<String>(value) {
        return Ok(value);
    }
    if value.starts_with('\'') {
        let inner = value
            .strip_prefix('\'')
            .and_then(|v| v.strip_suffix('\''))
            .ok_or_else(|| {
                SkillError::InvalidFrontmatter("chuỗi single-quote chưa đóng".to_string())
            })?;
        return Ok(inner.replace("''", "'"));
    }
    if value.starts_with('"') {
        return Err(SkillError::InvalidFrontmatter(
            "chuỗi double-quote không hợp lệ".to_string(),
        ));
    }
    Ok(value
        .split_once(" #")
        .map_or(value, |(before, _)| before)
        .trim()
        .to_string())
}

fn normalize_description(description: &str) -> Result<String, SkillError> {
    let description = description.trim();
    if description.is_empty() {
        return Err(SkillError::InvalidDescription("rỗng".to_string()));
    }
    if description.chars().any(|c| c == '\n' || c == '\r') {
        return Err(SkillError::InvalidDescription(
            "phải là một dòng".to_string(),
        ));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(SkillError::InvalidDescription(format!(
            "dài {} ký tự, tối đa {MAX_DESCRIPTION_CHARS}",
            description.chars().count()
        )));
    }
    Ok(description.to_string())
}

/// Kiểm tra kebab-case ASCII và từ chối mọi ký tự có thể tạo thành đường dẫn.
pub fn is_kebab_case(name: &str) -> bool {
    if name.is_empty() || name.starts_with('-') || name.ends_with('-') {
        return false;
    }
    let mut previous_dash = false;
    for ch in name.chars() {
        if ch == '-' {
            if previous_dash {
                return false;
            }
            previous_dash = true;
        } else if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            previous_dash = false;
        } else {
            return false;
        }
    }
    !previous_dash
}

fn validate_name(name: &str) -> Result<(), SkillError> {
    if is_kebab_case(name) {
        Ok(())
    } else {
        Err(SkillError::InvalidName(format!(
            "`{name}` phải là kebab-case ASCII, không chứa ký tự đường dẫn"
        )))
    }
}
