//! Vòng lặp agent (agents.md mục 6, 7.2, 15.3, 15.4, 15.8).
//!
//! # Module (tách theo trách nhiệm)
//! - `run` — vòng lặp [`Agent::run`] + thông báo kết thúc lượt
//! - `tool_call` — [`Agent::run_tool_call`], audit, cắt output tool
//! - `error` — [`AgentError`]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::store::{Store, StoreError};
use bean_llm::{LlmError, LlmProvider};
use bean_memory::{
    ensure_daily_budget, ensure_role_daily_budget, record_usage, record_usage_for_role,
};
use bean_security::audit::{AuditEntry, AuditLog, entry_now};
use bean_security::policy::{PolicyDecision, SessionPolicy, decide, deny_list_reason};
use bean_security::untrusted::contains_untrusted_block;
use bean_tools::{AlertSink, ToolCtx, ToolError, ToolOutput};
use bean_types::{
    Config, LlmDelta, LlmResponse, Message, Risk, RolePermissions, StopReason, ToolCall, ToolSpec,
    Usage,
};
use futures_util::StreamExt;
use tokio::time::timeout;

use crate::run_io::{Decision, RunIo};

const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;
const CANCELLED_MSG: &str = "[bị người dùng huỷ]";
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(300);
/// Trần số tool `Safe` chạy đồng thời trong một nhóm song song — model có thể
/// phát cả chục call trong một lượt; không chặn sẽ dồn hàng chục đọc file/grep
/// lên máy cùng lúc (lộ trình mục 5a).
const MAX_PARALLEL_TOOL_CALLS: usize = 4;
type RepeatKey = (String, String);

mod error;
mod parallel;
mod run;
mod tool_call;

pub use error::AgentError;

/// Vỏ tương thích ngược cho [`run_turn`] / [`run_turn_outcome`].
///
/// Từ bản refactor OOP, phụ thuộc dài hạn nằm trong [`Agent`] và dữ liệu mỗi lượt
/// nằm trong [`Turn`]; struct này chỉ còn giữ API cũ (test + adapter) hoạt động nên
/// các trường vẫn `pub` như trước. Code mới nên dựng [`Agent`] + [`Turn`] trực tiếp.
pub struct RunTurnArgs<'a> {
    /// Store lịch sử hội thoại.
    pub store: &'a dyn Store,
    /// Registry tool đã đăng ký.
    pub registry: &'a bean_tools::ToolRegistry,
    /// Provider LLM.
    pub llm: &'a dyn LlmProvider,
    /// Cấu hình chung.
    pub config: &'a Config,
    /// Phiên hội thoại.
    pub session: bean_types::SessionId,
    /// Tin nhắn người dùng mở đầu lượt.
    pub user_text: String,
    /// Kênh I/O (hiển thị tiến trình, xin xác nhận).
    pub io: Arc<dyn RunIo>,
    /// Token huỷ run đang chạy.
    pub cancel: tokio_util::sync::CancellationToken,
    /// Trạng thái "cho phép tool này trong phiên" — truyền `Arc` dùng chung qua các
    /// turn của cùng phiên; `None` ⇒ mỗi lượt lại hỏi (mục 7.2). — M4.
    pub session_policy: Option<Arc<SessionPolicy>>,
    /// Audit log JSONL (mục 15.8); `None` ⇒ không ghi (test/demo). — M4.
    pub audit: Option<Arc<AuditLog>>,
    /// Kênh của lượt, dùng cho audit (`"cli"` | `"web"` | `"telegram"` | `"scheduler"`).
    pub channel: &'a str,
    /// Progressive-disclosure index `name: description` của các skill đang có.
    pub skills_index: &'a str,
    /// Quyền RBAC **đã resolve** cho người gửi (M21.5).
    ///
    /// Router gọi `Config::permissions_for` **một lần** rồi truyền struct tuần tự hoá được này
    /// xuống đây. Agent loop **không** tự tra cứu role ⇒ quyết định RBAC nằm đúng một chỗ
    /// (ràng buộc `Plan.md` mục 4.3), và struct thì tuần tự hoá được nên sẵn sàng cho mô hình
    /// nhiều tiến trình sau này (mục 4.1).
    pub permissions: &'a RolePermissions,
    /// Project profile của lượt này (M21.1); quyết định workspace và file bộ nhớ nạp.
    pub project: &'a str,
    /// Kênh gửi cảnh báo chủ động cho tool (M23); `None` ⇒ tool chỉ trả cảnh báo trong kết quả.
    pub alerts: Option<Arc<dyn AlertSink>>,
}

/// Tập **phụ thuộc bất biến** của vòng lặp agent (agents.md mục 6).
///
/// # Vì sao tách khỏi [`RunTurnArgs`]
///
/// `RunTurnArgs` cũ gom cả phụ thuộc dài hạn (store, provider, registry, config) lẫn
/// dữ liệu riêng của từng lượt (session, tin nhắn, io, huỷ) vào một struct 16 trường
/// toàn `pub` — vừa hở đóng gói vừa trộn hai vòng đời khác nhau.
///
/// `Agent` giữ phần **bất biến theo tiến trình** (đóng gói, trường private), còn
/// [`Turn`] giữ phần **thay đổi mỗi lượt**. Nhờ đó vòng lặp là một *đối tượng* có
/// trạng thái ổn định thay vì một hàm tự do nhận 16 tham số, và [`RunTurnArgs`] chỉ
/// còn là vỏ tương thích ngược.
pub struct Agent<'a> {
    /// Store lịch sử hội thoại.
    store: &'a dyn Store,
    /// Registry tool đã đăng ký.
    registry: &'a bean_tools::ToolRegistry,
    /// Provider LLM.
    llm: &'a dyn LlmProvider,
    /// Cấu hình chung.
    config: &'a Config,
    /// Audit log JSONL (mục 15.8); `None` ⇒ không ghi (test/demo).
    audit: Option<Arc<AuditLog>>,
    /// Progressive-disclosure index `name: description` của các skill đang có.
    skills_index: &'a str,
    /// Kênh gửi cảnh báo chủ động cho tool (M23); `None` ⇒ tool chỉ trả cảnh báo.
    alerts: Option<Arc<dyn AlertSink>>,
}

/// Dữ liệu **riêng của một lượt** — phần thay đổi theo từng lần gọi.
///
/// Tách khỏi [`Agent`] để hai khái niệm không trộn: `Agent` sống cùng tiến trình,
/// `Turn` chỉ sống trong một lần gọi (xem [`Agent`] để biết lý do).
pub struct Turn<'a> {
    /// Phiên hội thoại.
    pub session: bean_types::SessionId,
    /// Tin nhắn người dùng mở đầu lượt.
    pub user_text: String,
    /// Kênh I/O (hiển thị tiến trình, xin xác nhận).
    pub io: Arc<dyn RunIo>,
    /// Token huỷ run đang chạy.
    pub cancel: tokio_util::sync::CancellationToken,
    /// Trạng thái "cho phép tool này trong phiên"; `None` ⇒ mỗi lượt hỏi lại (mục 7.2).
    pub session_policy: Option<Arc<SessionPolicy>>,
    /// Kênh của lượt, dùng cho audit (`"cli"` | `"web"` | `"telegram"` | `"scheduler"`).
    pub channel: &'a str,
    /// Quyền RBAC **đã resolve** cho người gửi (M21.5).
    pub permissions: &'a RolePermissions,
    /// Project profile của lượt này (M21.1).
    pub project: &'a str,
}

/// Trạng thái **thay đổi** trong một lượt — nhóm các biến `&mut` của vòng lặp.
///
/// Tách riêng khỏi [`Agent`] (bất biến theo tiến trình) và [`Turn`] (đầu vào của lượt)
/// để vòng lặp không phải truyền tay hàng chục tham số `&mut`. Đây cũng là ranh giới
/// tự nhiên để rút [`Agent::run_tool_call`] khỏi [`Agent::run`] mà không đụng logic.
///
/// # Chạy song song (lộ trình mục 5a)
///
/// Nhóm tool `Safe` liên tiếp chạy đồng thời nên các phần trạng thái dùng chung nằm
/// sau `Arc<tokio::sync::Mutex>` (khoá chờ async, không poisoning): từng task tool
/// cầm `&TurnState` bất biến và tự đồng bộ hoá. Đoạn găng (RBAC, runtime toggle,
/// chống lặp) vẫn chạy tuần tự theo thứ tự call trong response; các lần ghi sau
/// thực thi (đếm thất bại, cờ untrusted, transcript) tới khi từng task hoàn tất —
/// thứ tự hoàn tất trong nhóm song song là không xác định, nhưng **mọi** kết quả
/// đều được ghi và context builder chuẩn hoá lại thứ tự tool result theo thứ tự
/// `tool_calls` của assistant (xem `context::stabilize_tool_order`).
#[derive(Debug, Default)]
struct TurnState {
    /// Lịch sử của lượt (mirror DB; đồng thời là nguồn sự thật cho `RunOutcome`).
    transcript: Arc<tokio::sync::Mutex<Vec<Message>>>,
    /// Tổng số tool call đã chạy trong lượt (báo cáo + chống lặp).
    ///
    /// Chỉ [`Agent::run`] ghi (tuần tự trước khi chạy từng call/nhóm) nên không cần khoá.
    tool_call_count: usize,
    /// Skill đã nạp qua `load_skill` trong lượt này (đưa vào `RunOutcome`).
    loaded_skills: Arc<tokio::sync::Mutex<BTreeSet<String>>>,
    /// Số lần `(tool, args)` thất bại liên tiếp (chống lặp, agents.md mục 6).
    failure_counts: Arc<tokio::sync::Mutex<HashMap<RepeatKey, u32>>>,
    /// `(tool, args)` vừa thất bại — cờ "đang lặp ngay" cho lần gọi kế tiếp.
    consecutive_same_failure: Arc<tokio::sync::Mutex<Option<RepeatKey>>>,
    /// Cờ "lượt này đã đọc nội dung không tin cậy" (mục 15.4), dùng chung cho **mọi**
    /// tool trong lượt: một tool result chứa `<untrusted_content>` ⇒ mọi tool `Confirm`
    /// trở lên phải hỏi lại.
    untrusted_seen: Arc<AtomicBool>,
    /// Tool vừa thất bại 2 lần giống hệt nhau ⇒ kết thúc run bằng [`AgentError::RepeatFailure`].
    repeated_tool: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl TurnState {
    /// Rút toàn bộ transcript ra khi kết thúc lượt — mirror không dùng nữa.
    async fn take_transcript(&self) -> Vec<Message> {
        std::mem::take(&mut *self.transcript.lock().await)
    }

    /// Snapshot skill đã nạp trong lượt (kết thúc lượt).
    async fn loaded_skills_snapshot(&self) -> BTreeSet<String> {
        self.loaded_skills.lock().await.clone()
    }
}

/// Lý do run kết thúc bình thường.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// Provider trả lời cuối.
    Final,
    /// Đạt giới hạn bước.
    MaxSteps,
    /// Đã chạm hoặc vượt ngân sách token/ngày.
    BudgetExceeded,
    /// Provider kết thúc lượt nhưng **không trả về chữ nào** — run vẫn ghi vào lịch
    /// sử, nhưng kèm thông báo nói rõ chuyện gì xảy ra thay vì im lặng.
    EmptyResponse,
}

/// Kết quả đầy đủ cho Router; wrapper [`run_turn`] chỉ trả text để tương thích M3.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutcome {
    /// Text cuối để hiển thị.
    pub text: String,
    /// Message assistant cuối nếu có.
    pub message_id: Option<i64>,
    /// Lý do kết thúc.
    pub ended: EndReason,
    /// Tổng số tool call phát sinh trong lượt này.
    pub tool_call_count: usize,
    /// Tên skill được load thành công trong lượt này.
    pub loaded_skills: Vec<String>,
    /// Chỉ chứa message của lượt này, dùng làm bằng chứng cho reflection.
    pub transcript: Vec<Message>,
    /// Audio cuối cùng dưới dạng base64, nếu có (push-to-talk).
    pub audio_base64: Option<String>,
    /// MIME type của audio, nếu có.
    pub mime: Option<String>,
}

/// Chạy lượt và chỉ trả text (API tương thích M3–M7).
pub async fn run_turn(args: RunTurnArgs<'_>) -> Result<String, AgentError> {
    run_turn_outcome(args).await.map(|outcome| outcome.text)
}

/// Chạy lượt và trả outcome đầy đủ cho Router.
///
/// Vỏ tương thích ngược: dựng [`Agent`] + [`Turn`] rồi giao cho [`Agent::run`].
pub async fn run_turn_outcome(args: RunTurnArgs<'_>) -> Result<RunOutcome, AgentError> {
    let RunTurnArgs {
        store,
        registry,
        llm,
        config,
        session,
        user_text,
        io,
        cancel,
        session_policy,
        audit,
        channel,
        skills_index,
        permissions,
        project,
        alerts,
    } = args;
    Agent {
        store,
        registry,
        llm,
        config,
        audit,
        skills_index,
        alerts,
    }
    .run(Turn {
        session,
        user_text,
        io,
        cancel,
        session_policy,
        channel,
        permissions,
        project,
    })
    .await
}

async fn append_run_message(
    store: &dyn Store,
    session: bean_types::SessionId,
    transcript: &tokio::sync::Mutex<Vec<Message>>,
    message: Message,
) -> Result<i64, crate::store::StoreError> {
    // Ghi DB **trước** rồi mới push mirror: khoá chỉ giữ cho lần push đồng bộ,
    // không giữ qua `.await` (mục 22.7). Thứ tự ghi DB của các tool result chạy
    // song song là thứ tự hoàn tất; context builder chuẩn hoá lại theo tool_use.
    let id = store.append(session, message.clone()).await?;
    transcript.lock().await.push(message);
    Ok(id)
}
