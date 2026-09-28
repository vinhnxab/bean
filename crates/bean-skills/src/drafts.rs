//! Skill draft M15: lưu đề xuất chờ duyệt, tạo diff và kích hoạt lại loader.
//!
//! Mọi API ở module này là blocking; Router và REST phải gọi qua `spawn_blocking`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::loader::{Skill, SkillCatalog, SkillError, is_kebab_case, render_skill};

const DRAFT_METADATA_FILE: &str = "metadata.json";
const DRAFT_STATE_FILE: &str = ".state.json";
const MAX_REASON_CHARS: usize = 500;
const MAX_METADATA_BYTES: usize = 250_000;

/// Draft tạo skill mới hay sửa một skill đã được nạp trong run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDraftKind {
    /// Đề xuất skill mới.
    New,
    /// Đề xuất sửa skill đã tồn tại.
    Update,
}

impl SkillDraftKind {
    /// Chuỗi ổn định cho REST.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Update => "update",
        }
    }
}

/// Trạng thái quyết định của draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDraftStatus {
    /// Đang chờ người dùng duyệt.
    Pending,
    /// Đã kích hoạt.
    Approved,
    /// Người dùng đã bỏ.
    Rejected,
}

impl SkillDraftStatus {
    /// Chuỗi ổn định cho REST.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// Dữ liệu core để tạo một draft.
#[derive(Debug, Clone)]
pub struct NewSkillDraft {
    /// Tên skill mới hoặc skill cần sửa.
    pub name: String,
    /// Kiểu đề xuất.
    pub kind: SkillDraftKind,
    /// Description được đề xuất cho skill mới hoặc bản sửa.
    pub description: String,
    /// Body markdown không gồm frontmatter.
    pub body: String,
    /// Lý do ngắn để người dùng quyết định.
    pub reason: String,
    /// Session tạo ra đề xuất.
    pub source_session_id: i64,
    /// Channel nhận thông báo.
    pub source_channel: String,
    /// Chat ID nhận thông báo.
    pub source_chat_id: String,
    /// Timestamp RFC3339 UTC.
    pub created_at: String,
}

/// Snapshot một draft để REST hiển thị.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDraft {
    /// ID ngẫu nhiên.
    pub id: String,
    /// Tên skill.
    pub name: String,
    /// Loại đề xuất.
    pub kind: SkillDraftKind,
    /// Description được parse từ SKILL.md hợp lệ.
    pub description: String,
    /// Nội dung đầy đủ với skill mới, unified diff với skill sửa.
    pub content: String,
    /// Lý do reflection đưa ra.
    pub reason: String,
    /// Trạng thái hiện tại.
    pub status: SkillDraftStatus,
    /// Timestamp tạo.
    pub created_at: String,
    pub(crate) proposed_content: String,
    pub(crate) base_content: Option<String>,
    pub(crate) source_session_id: i64,
    pub(crate) source_channel: String,
    pub(crate) source_chat_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftMetadata {
    version: u8,
    id: String,
    name: String,
    kind: SkillDraftKind,
    reason: String,
    created_at: String,
    base_content: Option<String>,
    source_session_id: i64,
    source_channel: String,
    source_chat_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DraftState {
    last_proposed_at: Option<String>,
}

struct DraftInner {
    operation: Mutex<()>,
    state: Mutex<DraftState>,
    state_error: Mutex<Option<String>>,
}

/// State chia sẻ giữa các clone của [`SkillCatalog`].
#[derive(Clone)]
pub(crate) struct DraftRuntime {
    inner: Arc<DraftInner>,
}

impl DraftRuntime {
    pub(crate) fn load(draft_root: &Path) -> Self {
        let path = draft_root.join(DRAFT_STATE_FILE);
        let (state, state_error) = match read_state(&path) {
            Ok(state) => (state, None),
            Err(error) => {
                warn!(path = %path.display(), error = %error,
                    "không đọc được state learning; sẽ chặn đề xuất mới cho tới khi sửa file");
                (DraftState::default(), Some(error.to_string()))
            }
        };
        Self {
            inner: Arc::new(DraftInner {
                operation: Mutex::new(()),
                state: Mutex::new(state),
                state_error: Mutex::new(state_error),
            }),
        }
    }

    fn operation(&self) -> Result<MutexGuard<'_, ()>, SkillError> {
        self.inner
            .operation
            .lock()
            .map_err(|_| SkillError::CatalogPoisoned)
    }
}

impl SkillCatalog {
    /// Timestamp đề xuất đã tạo gần nhất, phục vụ rate limit bền qua restart.
    pub fn last_proposal_at(&self) -> Result<Option<String>, SkillError> {
        if let Some(error) = self
            .drafts
            .inner
            .state_error
            .lock()
            .map_err(|_| SkillError::CatalogPoisoned)?
            .as_ref()
        {
            return Err(SkillError::InvalidDraft(error.clone()));
        }
        self.drafts
            .inner
            .state
            .lock()
            .map(|state| state.last_proposed_at.clone())
            .map_err(|_| SkillError::CatalogPoisoned)
    }

    /// Liệt kê draft hợp lệ; file lỗi bị log và bỏ qua.
    pub fn list_drafts(&self) -> Result<Vec<SkillDraft>, SkillError> {
        let _operation = self.drafts.operation()?;
        let mut drafts = Vec::new();
        let entries = match fs::read_dir(&self.draft_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(drafts),
            Err(error) => return Err(SkillError::Io(error.to_string())),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            match read_draft(&path) {
                Ok(draft) => drafts.push(draft),
                Err(error) => warn!(path = %path.display(), error = %error,
                    "bỏ qua skill nháp không hợp lệ"),
            }
        }
        drafts.sort_by(|left, right| {
            let right_time = chrono::DateTime::parse_from_rfc3339(&right.created_at).ok();
            let left_time = chrono::DateTime::parse_from_rfc3339(&left.created_at).ok();
            right_time
                .cmp(&left_time)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(drafts)
    }

    /// Tạo draft hợp lệ dưới `skills/_drafts/<name>/`; chưa kích hoạt skill.
    pub fn create_draft(&self, input: NewSkillDraft) -> Result<SkillDraft, SkillError> {
        let _operation = self.drafts.operation()?;
        if !is_kebab_case(&input.name) {
            return Err(SkillError::InvalidName(input.name));
        }
        let reason = input.reason.trim().to_string();
        if reason.is_empty() || reason.chars().count() > MAX_REASON_CHARS {
            return Err(SkillError::InvalidDraft(format!(
                "lý do phải có 1..={MAX_REASON_CHARS} ký tự"
            )));
        }
        if input.source_session_id <= 0
            || input.source_channel.trim().is_empty()
            || input.source_chat_id.trim().is_empty()
            || chrono::DateTime::parse_from_rfc3339(&input.created_at).is_err()
        {
            return Err(SkillError::InvalidDraft(
                "source hoặc timestamp của draft không hợp lệ".into(),
            ));
        }
        let base_content = match input.kind {
            SkillDraftKind::New => match self.get(&input.name) {
                Ok(_) => return Err(SkillError::AlreadyExists(input.name)),
                Err(SkillError::NotFound(_)) => None,
                Err(error) => return Err(error),
            },
            SkillDraftKind::Update => Some(self.get(&input.name)?.content),
        };
        let proposed_content = render_skill(&input.name, &input.description, &input.body)?;
        if input.kind == SkillDraftKind::Update
            && base_content.as_deref() == Some(proposed_content.as_str())
        {
            return Err(SkillError::InvalidDraft(
                "bản sửa không thay đổi skill nền".into(),
            ));
        }
        let id = new_draft_id()?;
        let metadata = DraftMetadata {
            version: 1,
            id: id.clone(),
            name: input.name.clone(),
            kind: input.kind,
            reason: reason.clone(),
            created_at: input.created_at.clone(),
            base_content: base_content.clone(),
            source_session_id: input.source_session_id,
            source_channel: input.source_channel.clone(),
            source_chat_id: input.source_chat_id.clone(),
        };

        fs::create_dir_all(&self.draft_root).map_err(|error| SkillError::Io(error.to_string()))?;
        let root_metadata = fs::symlink_metadata(&self.draft_root)
            .map_err(|error| SkillError::Io(error.to_string()))?;
        if !root_metadata.file_type().is_dir() {
            return Err(SkillError::InvalidDraft(
                "_drafts phải là thư mục thật".into(),
            ));
        }
        let directory = self.draft_root.join(&input.name);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(SkillError::DraftAlreadyPending(input.name));
            }
            Err(error) => return Err(SkillError::Io(error.to_string())),
        }
        let cleanup = || {
            let _ = fs::remove_file(directory.join(crate::loader::SKILL_FILE));
            let _ = fs::remove_file(directory.join(DRAFT_METADATA_FILE));
            let _ = fs::remove_dir(&directory);
        };
        if let Err(error) = create_new_file(
            &directory.join(crate::loader::SKILL_FILE),
            proposed_content.as_bytes(),
        ) {
            cleanup();
            return Err(error);
        }
        let metadata_json = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| SkillError::InvalidDraft(error.to_string()))?;
        if let Err(error) = create_new_file(&directory.join(DRAFT_METADATA_FILE), &metadata_json) {
            cleanup();
            return Err(error);
        }
        let proposed = match SkillCatalog::read_skill(&directory) {
            Ok(skill) => skill,
            Err(error) => {
                cleanup();
                return Err(error);
            }
        };
        if let Err(error) = self.persist_proposal_time(&input.created_at) {
            cleanup();
            return Err(error);
        }
        Ok(SkillDraft {
            id,
            name: input.name,
            kind: input.kind,
            description: proposed.description,
            content: display_content(input.kind, base_content.as_deref(), &proposed_content),
            reason,
            status: SkillDraftStatus::Pending,
            created_at: input.created_at,
            proposed_content,
            base_content,
            source_session_id: input.source_session_id,
            source_channel: input.source_channel,
            source_chat_id: input.source_chat_id,
        })
    }
}

/// Kết quả duyệt/bỏ một draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDraftDecision {
    /// ID vừa xử lý.
    pub id: String,
    /// Tên skill.
    pub name: String,
    /// Trạng thái mới.
    pub status: SkillDraftStatus,
}

impl SkillCatalog {
    /// Duyệt draft: cài skill mới hoặc cập nhật skill nền, sau đó reload catalog.
    pub fn approve_draft(&self, id: &str) -> Result<SkillDraftDecision, SkillError> {
        let _operation = self.drafts.operation()?;
        let draft = self.find_draft(id)?;
        match draft.kind {
            SkillDraftKind::New => {
                match self.get(&draft.name) {
                    Ok(_) => return Err(SkillError::AlreadyExists(draft.name)),
                    Err(SkillError::NotFound(_)) => {}
                    Err(error) => return Err(error),
                }
                install_new_skill(self, &draft.name, &draft.proposed_content)?;
                if let Err(error) = self.reload().and_then(|()| {
                    self.get(&draft.name).map(|_| ()).map_err(|_| {
                        SkillError::InvalidDraft("skill mới không nạp được sau khi duyệt".into())
                    })
                }) {
                    let _ = fs::remove_file(
                        self.create_root
                            .join(&draft.name)
                            .join(crate::loader::SKILL_FILE),
                    );
                    let _ = fs::remove_dir(self.create_root.join(&draft.name));
                    let _ = self.reload();
                    return Err(error);
                }
            }
            SkillDraftKind::Update => {
                let current = self.get(&draft.name)?;
                if current.content != draft.base_content.as_deref().unwrap_or_default() {
                    return Err(SkillError::DraftConflict(draft.name));
                }
                atomic_write(
                    &current.directory.join(crate::loader::SKILL_FILE),
                    draft.proposed_content.as_bytes(),
                )?;
                if let Err(error) = self.reload().and_then(|()| {
                    self.get(&draft.name).map(|_| ()).map_err(|_| {
                        SkillError::InvalidDraft("bản sửa không nạp được sau khi duyệt".into())
                    })
                }) {
                    let _ = atomic_write(
                        &current.directory.join(crate::loader::SKILL_FILE),
                        current.content.as_bytes(),
                    );
                    let _ = self.reload();
                    return Err(error);
                }
            }
        }
        if let Err(error) = remove_draft_directory(&draft.name, &self.draft_root) {
            match draft.kind {
                SkillDraftKind::New => {
                    let _ = fs::remove_file(
                        self.create_root
                            .join(&draft.name)
                            .join(crate::loader::SKILL_FILE),
                    );
                    let _ = fs::remove_dir(self.create_root.join(&draft.name));
                }
                SkillDraftKind::Update => {
                    if let Ok(current) = self.get(&draft.name) {
                        let _ = atomic_write(
                            &current.directory.join(crate::loader::SKILL_FILE),
                            draft.base_content.as_deref().unwrap_or_default().as_bytes(),
                        );
                    }
                }
            }
            let _ = self.reload();
            return Err(error);
        }
        Ok(SkillDraftDecision {
            id: draft.id,
            name: draft.name,
            status: SkillDraftStatus::Approved,
        })
    }

    /// Bỏ draft; không thay đổi skill đang hoạt động.
    pub fn reject_draft(&self, id: &str) -> Result<SkillDraftDecision, SkillError> {
        let _operation = self.drafts.operation()?;
        let draft = self.find_draft(id)?;
        remove_draft_directory(&draft.name, &self.draft_root)?;
        Ok(SkillDraftDecision {
            id: draft.id,
            name: draft.name,
            status: SkillDraftStatus::Rejected,
        })
    }

    fn persist_proposal_time(&self, created_at: &str) -> Result<(), SkillError> {
        if let Some(error) = self
            .drafts
            .inner
            .state_error
            .lock()
            .map_err(|_| SkillError::CatalogPoisoned)?
            .as_ref()
        {
            return Err(SkillError::InvalidDraft(error.clone()));
        }
        let mut state = self
            .drafts
            .inner
            .state
            .lock()
            .map_err(|_| SkillError::CatalogPoisoned)?;
        let mut updated = (*state).clone();
        updated.last_proposed_at = Some(created_at.to_string());
        write_state(&self.draft_root.join(DRAFT_STATE_FILE), &updated)?;
        *state = updated;
        Ok(())
    }

    fn find_draft(&self, id: &str) -> Result<SkillDraft, SkillError> {
        let entries = match fs::read_dir(&self.draft_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(SkillError::DraftNotFound(id.to_string()));
            }
            Err(error) => return Err(SkillError::Io(error.to_string())),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            if let Ok(draft) = read_draft(&path)
                && draft.id == id
            {
                return Ok(draft);
            }
        }
        Err(SkillError::DraftNotFound(id.to_string()))
    }
}

fn read_draft(directory: &Path) -> Result<SkillDraft, SkillError> {
    let metadata =
        fs::symlink_metadata(directory).map_err(|error| SkillError::Io(error.to_string()))?;
    if !metadata.file_type().is_dir() {
        return Err(SkillError::InvalidDraft(
            "thư mục draft không phải thư mục thật".into(),
        ));
    }
    let directory_name = directory
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| SkillError::InvalidDraft("tên thư mục draft không UTF-8".into()))?;
    let metadata_path = directory.join(DRAFT_METADATA_FILE);
    let metadata_file =
        fs::symlink_metadata(&metadata_path).map_err(|error| SkillError::Io(error.to_string()))?;
    if !metadata_file.file_type().is_file() {
        return Err(SkillError::InvalidDraft(
            "metadata draft không phải file thường".into(),
        ));
    }
    let raw = fs::read(&metadata_path).map_err(|error| SkillError::Io(error.to_string()))?;
    if raw.len() > MAX_METADATA_BYTES {
        return Err(SkillError::InvalidDraft(
            "metadata draft vượt giới hạn kích thước".into(),
        ));
    }
    let metadata: DraftMetadata = serde_json::from_slice(&raw)
        .map_err(|error| SkillError::InvalidDraft(error.to_string()))?;
    if metadata.version != 1 || !is_valid_draft_id(&metadata.id) || metadata.name != directory_name
    {
        return Err(SkillError::InvalidDraft(
            "metadata draft không khớp thư mục".into(),
        ));
    }
    if metadata.reason.trim().is_empty()
        || metadata.reason.chars().count() > MAX_REASON_CHARS
        || metadata.source_session_id <= 0
        || metadata.source_channel.trim().is_empty()
        || metadata.source_chat_id.trim().is_empty()
        || chrono::DateTime::parse_from_rfc3339(&metadata.created_at).is_err()
    {
        return Err(SkillError::InvalidDraft(
            "metadata draft thiếu trường bắt buộc".into(),
        ));
    }
    let proposed = SkillCatalog::read_skill(directory)?;
    match (metadata.kind, metadata.base_content.as_ref()) {
        (SkillDraftKind::New, None) => {}
        (SkillDraftKind::Update, Some(_)) => {}
        _ => {
            return Err(SkillError::InvalidDraft(
                "loại draft không khớp nội dung skill nền".into(),
            ));
        }
    }
    Ok(SkillDraft {
        id: metadata.id,
        name: metadata.name,
        kind: metadata.kind,
        description: proposed.description,
        content: display_content(
            metadata.kind,
            metadata.base_content.as_deref(),
            &proposed.content,
        ),
        reason: metadata.reason,
        status: SkillDraftStatus::Pending,
        created_at: metadata.created_at,
        proposed_content: proposed.content,
        base_content: metadata.base_content,
        source_session_id: metadata.source_session_id,
        source_channel: metadata.source_channel,
        source_chat_id: metadata.source_chat_id,
    })
}

fn read_state(path: &Path) -> Result<DraftState, SkillError> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DraftState::default());
        }
        Err(error) => return Err(SkillError::Io(error.to_string())),
    };
    let state: DraftState = serde_json::from_slice(&raw)
        .map_err(|error| SkillError::InvalidDraft(error.to_string()))?;
    if state
        .last_proposed_at
        .as_deref()
        .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
    {
        return Err(SkillError::InvalidDraft(
            "state learning có timestamp không hợp lệ".into(),
        ));
    }
    Ok(state)
}

fn write_state(path: &Path, state: &DraftState) -> Result<(), SkillError> {
    let raw = serde_json::to_vec_pretty(state)
        .map_err(|error| SkillError::InvalidDraft(error.to_string()))?;
    atomic_write(path, &raw)
}

fn new_draft_id() -> Result<String, SkillError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| SkillError::DraftRandom(error.to_string()))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(38);
    id.push_str("draft_");
    for byte in bytes {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(id)
}

/// Kiểm tra định dạng ID để Telegram giới hạn callback data.
#[must_use]
pub fn is_valid_draft_id(value: &str) -> bool {
    value.len() == 38
        && value.starts_with("draft_")
        && value
            .as_bytes()
            .get(6..)
            .is_some_and(|suffix| suffix.len() == 32 && suffix.iter().all(u8::is_ascii_hexdigit))
}

fn create_new_file(path: &Path, bytes: &[u8]) -> Result<(), SkillError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| SkillError::Io(error.to_string()))?;
    file.write_all(bytes)
        .map_err(|error| SkillError::Io(error.to_string()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), SkillError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillError::Io(error.to_string()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| SkillError::Io("tên file không UTF-8".into()))?;
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    if let Ok(metadata) = fs::symlink_metadata(&temporary)
        && metadata.file_type().is_file()
    {
        let _ = fs::remove_file(&temporary);
    }
    create_new_file(&temporary, bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(SkillError::Io(error.to_string()));
    }
    Ok(())
}

fn install_new_skill(
    catalog: &SkillCatalog,
    name: &str,
    content: &str,
) -> Result<Skill, SkillError> {
    fs::create_dir_all(&catalog.create_root).map_err(|error| SkillError::Io(error.to_string()))?;
    let directory = catalog.create_root.join(name);
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(SkillError::AlreadyExists(name.to_string()));
        }
        Err(error) => return Err(SkillError::Io(error.to_string())),
    }
    if let Err(error) = create_new_file(
        &directory.join(crate::loader::SKILL_FILE),
        content.as_bytes(),
    ) {
        let _ = fs::remove_file(directory.join(crate::loader::SKILL_FILE));
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    SkillCatalog::read_skill(&directory)
}

fn remove_draft_directory(name: &str, draft_root: &Path) -> Result<(), SkillError> {
    let directory = draft_root.join(name);
    let metadata =
        fs::symlink_metadata(&directory).map_err(|error| SkillError::Io(error.to_string()))?;
    if !metadata.file_type().is_dir() {
        return Err(SkillError::InvalidDraft(
            "draft không phải thư mục thật".into(),
        ));
    }
    fs::remove_dir_all(directory).map_err(|error| SkillError::Io(error.to_string()))
}

fn display_content(kind: SkillDraftKind, base: Option<&str>, proposed: &str) -> String {
    match kind {
        SkillDraftKind::New => proposed.to_string(),
        SkillDraftKind::Update => unified_diff(base.unwrap_or_default(), proposed, "SKILL.md"),
    }
}

fn unified_diff(old: &str, new: &str, path: &str) -> String {
    if old == new {
        return "(không có thay đổi)\n".to_string();
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(left, right)| left == right)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(left, right)| left == right)
        .count();
    let old_end = old_lines.len().saturating_sub(suffix);
    let new_end = new_lines.len().saturating_sub(suffix);
    let mut out = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        prefix + 1,
        old_end - prefix,
        prefix + 1,
        new_end - prefix
    );
    for line in &old_lines[..prefix] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    for line in &old_lines[prefix..old_end] {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &new_lines[prefix..new_end] {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    for line in &old_lines[old_end..] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    out
}
