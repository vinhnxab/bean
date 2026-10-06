//! Router, Channel và slash command của lõi (agents.md mục 10).
//!
//! Run thuộc Router: adapter chỉ `submit` và nhận event. Vòng đời run không phụ
//! thuộc subscriber, nên WebSocket rớt không làm cancel tool hoặc confirm.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use anyhow::Result as AnyResult;
use bean_llm::LlmProvider;
use bean_security::{AuditLog, SessionPolicy};
use bean_skills::{SkillCatalog, SkillDraftDecision, SkillError};
use bean_tools::{AlertSink, ToolRegistry, truncate_chars};
use bean_types::{
    AgentReport, AgentStatus, Config, ConfirmId, ConfirmOutcome, NO_ACCESS_ROLE, Outbound, Risk,
    RolePermissions, RunEvent, RunId, SessionId,
};
use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use crate::agent::{RunOutcome, RunTurnArgs, run_turn_outcome};
use crate::learning::{ReflectionArgs, reflect};
use crate::run_io::Decision;
use crate::store::{Store, StoreError};

use alert::RouterAlertSink;
use confirm::{ConfirmRegistry, ConfirmRequest};
use io::RouterIo;
use learning::LearningGate;
use queue::{Enqueued, RunQueue};
use skills::SkillCoordinator;

mod admit;
mod alert;
mod command;
mod confirm;
mod decision;
mod dispatch;
mod io;
mod learning;
mod outbox;
mod queue;
mod run;
mod skills;
mod status;
mod types;

pub use types::{
    Channel, Incoming, PendingConfirmInfo, RouterDeps, RouterError, RouterOptions, RouterSnapshot,
    RunningInfo,
};

const DEFAULT_EVENT_CAPACITY: usize = 256;
const OUTBOX_BATCH_SIZE: usize = 50;
const PREVIEW_CHARS: usize = 2_000;
/// Số lượng tối đa ghi trong `summary` của `AgentReport` (mục đích chỉ là dòng tóm tắt).
const MAX_SUMMARY_COUNT: usize = 9;

#[derive(Debug, Clone)]
struct QueuedRun {
    run_id: RunId,
    session_id: SessionId,
    incoming: Incoming,
    cancel: CancellationToken,
    /// `Some` khi run do scheduler gọi; interactive run là `None`.
    background_allowed_tools: Option<Arc<HashSet<String>>>,
    /// Role đã resolve khi run được xếp hàng; `None` khi RBAC tắt.
    ///
    /// Mang theo bản thân job (thay vì resolve lại lúc `finish`) để khi run kế tiếp
    /// được kích hoạt, `ActiveRun` nhận đúng role mà lúc xếp hàng đã quyết định —
    /// không phụ thuộc cấu hình có bị sửa giữa đường.
    role: Option<String>,
}

#[derive(Debug)]
struct PendingConfirm {
    run_id: RunId,
    session_id: SessionId,
    actor: String,
    prompt: String,
    risk: Risk,
    allow_session_option: bool,
    timeout_seconds: u32,
    /// Role sở hữu run đang chờ duyệt; dùng để HUB gắn hàng đời duyệt về đúng agent.
    role: Option<String>,
    sender: oneshot::Sender<Confirmation>,
}

#[derive(Debug, Clone)]
struct Confirmation {
    decision: Decision,
    actor: String,
}

#[derive(Debug, Default)]
struct RouterState {
    /// Máy trạng thái lịch trình: hàng đợi theo session + run đang chạy.
    queue: RunQueue,
    /// Sổ xác nhận đang chờ (mục 15.3).
    confirms: HashMap<ConfirmId, PendingConfirm>,
    /// "Cho phép tool này trong phiên" theo session — chỉ trong RAM, không ghi DB
    /// (mục 10, `D1.7`).
    policies: HashMap<SessionId, Arc<SessionPolicy>>,
}

/// Kết quả [`Router::admit`]: `run_id` + `session` đã hợp lệ, kèm khoá submit.
///
/// Mang khoá theo mình (tên `_guard` cố ý) để khoá **sống tới hết thân hàm gọi** —
/// nhờ vậy `enqueue` vẫn nằm trong vùng tuần tự hoá, y như khi `submit` tự giữ khoá.
/// Nếu chỉ trả `run_id`/`session`, khoá sẽ rơi khi hàm `admit` trả về và `enqueue`
/// chạy ngoài vùng bảo đảm — mất đúng tính tuần tự mà khoá này sinh ra.
struct Admission<'a> {
    /// Giữ khoá `submission` sống cho tới khi lượt đã được xếp hàng.
    _guard: tokio::sync::MutexGuard<'a, ()>,
    /// Run ID vừa cấp.
    run_id: RunId,
    /// Session đã resolve cho lượt này.
    session: SessionId,
}

struct RouterInner {
    /// Cấu hình có thể đổi lúc chạy (`/model`).
    config: RwLock<Config>,
    store: Arc<dyn Store>,
    registry: Arc<ToolRegistry>,
    llm: Arc<dyn LlmProvider>,
    audit: Option<Arc<AuditLog>>,
    /// Catalog skill + index progressive-disclosure (mục 9) và duyệt nháp (M15).
    skills: SkillCoordinator,
    /// Cổng cooldown của learning loop (M15): giữ chỗ cho đề xuất skill sắp tới.
    learning_gate: LearningGate,
    options: RouterOptions,
    events: broadcast::Sender<RunEvent>,
    channels: RwLock<HashMap<String, Arc<dyn Channel>>>,
    state: Mutex<RouterState>,
    submission: AsyncMutex<()>,
    outbox_started: AtomicBool,
    shutdown: CancellationToken,
}

/// Router sở hữu queue, run, confirm, channel và outbox.
#[derive(Clone)]
pub struct Router {
    inner: Arc<RouterInner>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("event_capacity", &self.inner.options.event_capacity)
            .finish_non_exhaustive()
    }
}

fn remember_actor(slot: &Mutex<Option<String>>, actor: String) {
    if let Ok(mut current) = slot.lock() {
        *current = Some(actor);
    }
}

fn preview(text: &str) -> String {
    match truncate_chars(text, PREVIEW_CHARS) {
        Some((kept, _)) => kept.to_string(),
        None => text.to_string(),
    }
}

fn new_run_id() -> Result<RunId, RouterError> {
    random_id("run_").map(RunId::new)
}

fn new_confirm_id() -> Result<ConfirmId, RouterError> {
    random_id("confirm_").map(ConfirmId::new)
}

fn random_id(prefix: &str) -> Result<String, RouterError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| RouterError::Random(error.to_string()))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut id = String::with_capacity(prefix.len() + 32);
    id.push_str(prefix);
    for byte in bytes {
        id.push(char::from(HEX[usize::from(byte >> 4)]));
        id.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(id)
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, RouterError> {
    mutex.lock().map_err(|_| RouterError::StatePoisoned)
}

fn read_lock<T>(lock: &RwLock<T>) -> Result<RwLockReadGuard<'_, T>, RouterError> {
    lock.read().map_err(|_| RouterError::StatePoisoned)
}

fn write_lock<T>(lock: &RwLock<T>) -> Result<RwLockWriteGuard<'_, T>, RouterError> {
    lock.write().map_err(|_| RouterError::StatePoisoned)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}
