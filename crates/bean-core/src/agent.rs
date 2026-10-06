//! Vòng lặp agent (agents.md mục 6, 7.2, 15.3, 15.4, 15.8).

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
type RepeatKey = (String, String);

#[derive(Debug, Default)]
struct StreamResponseBuilder {
    text: String,
    text_delta_index: u32,
    emitted_text: bool,
    tool_calls: std::collections::BTreeMap<usize, (Option<String>, Option<String>, String)>,
    stop: Option<StopReason>,
    usage: Usage,
}

impl StreamResponseBuilder {
    fn push(&mut self, delta: LlmDelta, io: &dyn RunIo) {
        match delta {
            LlmDelta::Text { text } => {
                io.on_text_delta(&text, self.text_delta_index, !self.emitted_text);
                self.emitted_text = true;
                self.text_delta_index = self.text_delta_index.saturating_add(1);
                self.text.push_str(&text);
            }
            LlmDelta::ToolCall(call) => {
                let entry = self.tool_calls.entry(call.index).or_default();
                if call.id.is_some() {
                    entry.0 = call.id;
                }
                if call.name.is_some() {
                    entry.1 = call.name;
                }
                if let Some(arguments) = call.arguments_delta {
                    entry.2.push_str(&arguments);
                }
            }
            LlmDelta::Stop { reason } => self.stop = Some(reason),
            LlmDelta::Usage { usage } => self.usage = usage,
        }
    }

    fn finish(self) -> Result<LlmResponse, AgentError> {
        let stop = self.stop.ok_or_else(|| {
            AgentError::Llm("provider streaming kết thúc mà không có stop reason".to_string())
        })?;
        let mut tool_calls = Vec::with_capacity(self.tool_calls.len());
        for (index, (id, name, arguments)) in self.tool_calls {
            let id = id.ok_or_else(|| {
                AgentError::Llm(format!("tool call stream index {index} thiếu id"))
            })?;
            let name = name.ok_or_else(|| {
                AgentError::Llm(format!("tool call stream index {index} thiếu name"))
            })?;
            let args = if arguments.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&arguments).map_err(|error| {
                    AgentError::Llm(format!(
                        "arguments của tool call stream index {index} không phải JSON hợp lệ: {error}"
                    ))
                })?
            };
            tool_calls.push(ToolCall::new(id, name, args));
        }
        Ok(LlmResponse {
            text: (!self.text.is_empty()).then_some(self.text),
            tool_calls,
            stop,
            usage: self.usage,
        })
    }
}

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
#[derive(Debug, Default)]
struct TurnState {
    /// Lịch sử của lượt (mirror DB; đồng thời là nguồn sự thật cho `RunOutcome`).
    transcript: Vec<Message>,
    /// Tổng số tool call đã chạy trong lượt (báo cáo + chống lặp).
    tool_call_count: usize,
    /// Skill đã nạp qua `load_skill` trong lượt này (đưa vào `RunOutcome`).
    loaded_skills: BTreeSet<String>,
    /// Số lần `(tool, args)` thất bại liên tiếp (chống lặp, agents.md mục 6).
    failure_counts: HashMap<RepeatKey, u32>,
    /// `(tool, args)` vừa thất bại — cờ "đang lặp ngay" cho lần gọi kế tiếp.
    consecutive_same_failure: Option<RepeatKey>,
    /// Cờ "lượt này đã đọc nội dung không tin cậy" (mục 15.4), dùng chung cho **mọi**
    /// tool trong lượt: một tool result chứa `<untrusted_content>` ⇒ mọi tool `Confirm`
    /// trở lên phải hỏi lại.
    untrusted_seen: Arc<AtomicBool>,
    /// Tool vừa thất bại 2 lần giống hệt nhau ⇒ kết thúc run bằng [`AgentError::RepeatFailure`].
    repeated_tool: Option<String>,
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

impl<'a> Agent<'a> {
    /// Chạy vòng lặp agent cho đúng một lượt người dùng (agents.md mục 6).
    ///
    /// Nhận [`Turn`] (dữ liệu riêng của lượt); phụ thuộc dài hạn đã nằm trong `self`.
    ///
    /// # Errors
    /// [`AgentError`] khi store/provider lỗi, khi run bị huỷ, hoặc chống-lặp kích hoạt.
    pub async fn run(&self, turn: Turn<'_>) -> Result<RunOutcome, AgentError> {
        // `turn` **không** destructure: `run_tool_call` cần mượn `&turn` cho từng tool
        // call, nên `turn` phải còn nguyên. Các thành phần dùng ngay ở đây được
        // ràng buộc thành biến cục bộ (phần còn lại của thân hàm giữ nguyên).
        let session = turn.session;
        let permissions = turn.permissions;
        let project = turn.project;
        let io = &turn.io;
        let cancel = &turn.cancel;
        let store = self.store;
        let registry = self.registry;
        let llm = self.llm;
        let config = self.config;
        let skills_index = self.skills_index;
        // (M5/D8.10) System prompt chỉ đi qua `ChatRequest.system` — **không** nhân bản nó
        // thành message `User` (M3 từng làm vậy: tốn token gấp đôi cho phần system và dễ
        // bị model hiểu nhầm là câu lệnh của người dùng). Giữ `turn_input` để dựng lỡ
        // trường hợp lịch sử rỗng.
        let turn_input = turn.user_text.clone();
        // Mọi biến `&mut` của lượt gom ở `TurnState` — kể cả cờ `untrusted_seen`
        // dùng chung cho MỌI tool trong lượt (mục 15.4).
        let mut state = TurnState::default();
        append_run_message(
            store,
            session,
            &mut state.transcript,
            Message::user(turn_input.clone()),
        )
        .await?;
        match store.compact(session, llm, config).await {
            Ok(()) => {}
            Err(StoreError::BudgetExceeded { used, limit }) => {
                return finish_with_notice(
                    store,
                    session,
                    state.transcript,
                    budget_notice(used, limit),
                    EndReason::BudgetExceeded,
                    state.tool_call_count,
                    state.loaded_skills,
                )
                .await;
            }
            Err(error) => return Err(error.into()),
        }
        // Session policy dùng chung; không truyền vào thì mỗi lượt hỏi lại (an toàn mặc định).
        let local_session_policy;
        let session_policy: &SessionPolicy = match turn.session_policy.as_ref() {
            Some(p) => p.as_ref(),
            None => {
                local_session_policy = SessionPolicy::new();
                &local_session_policy
            }
        };

        for _step in 0..config.agent.max_steps {
            // (M5, mục 8.2) Dựng context: system prompt + MEMORY.md/USER.md + summary của phiên
            // + lịch sử vừa ngân sách token, cắt ở ranh giới an toàn (không tách cặp tool).
            //
            // (M21.1) `MEMORY.md`/`USER.md` đọc từ workspace **của project**, và (M21.7) ngân
            // sách context lấy theo role nên role có thể có context budget riêng.
            let workspace = registry.workspace_for(project);
            let ctx = crate::context::build_for_project(
                store,
                config,
                session,
                workspace.as_deref(),
                skills_index,
                config.context_budget_for(permissions),
                &permissions.role,
            )
            .await?;
            let system = ctx.system;
            // (K1, `D9.7`) Phiên đã compact ⇒ context mang `sessions.summary`, tức dữ liệu
            // tổng hợp từ lịch sử có thể chứa nội dung không tin cậy. Bật cờ **ngay từ đầu
            // lượt** để mọi tool `Confirm` trở lên phải hỏi lại ngay cả khi lượt này không
            // chạy tool đọc nội dung nào (mục 15.4 yêu cầu hai điều kiện kèm nhau: bọc thẻ
            // VÀ bật cờ — bọc thẻ một mình chỉ là soft control).
            if ctx.summary_present {
                state.untrusted_seen.store(true, Ordering::SeqCst);
            }
            let mut messages = ctx.messages;
            if messages.is_empty() {
                // Provider (Anthropic) từ chối `messages: []`. Sau `append` ở trên lịch sử
                // không bao giờ rỗng — đây chỉ là lưới an toàn, không phải đường đi bình thường.
                messages.push(Message::user(turn_input.clone()));
            }

            let usage_day = chrono::Utc::now().format("%Y-%m-%d").to_string();
            // (M21.7) Hai ngân sách độc lập: **tổng** toàn instance (mục 15.9) và **riêng role**
            // này. Nhờ vậy Developer chạy vòng lặp dài không ăn hết hạn mức khiến
            // Monitor/Security-scan không chạy được job định kỳ.
            let role_budget = config.daily_budget_for(permissions);
            for budget_error in [
                ensure_daily_budget(store, &usage_day, config.security.daily_token_budget).await,
                ensure_role_daily_budget(store, &usage_day, permissions.usage_scope(), role_budget)
                    .await,
            ] {
                match budget_error {
                    Ok(()) => {}
                    Err(StoreError::BudgetExceeded { used, limit }) => {
                        return finish_with_notice(
                            store,
                            session,
                            state.transcript,
                            budget_notice(used, limit),
                            EndReason::BudgetExceeded,
                            state.tool_call_count,
                            state.loaded_skills,
                        )
                        .await;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            // (M21.5) **Lọc danh sách tool TRƯỚC khi dựng request tới LLM** — không lọc sau khi
            // model đã "chọn" tool. Model không thấy tool ngoài quyền của role nên không thể gọi
            // nhầm, và cũng không bị dắt vào hướng dẫn bằng schema của tool bị cấm.
            let tool_specs: Vec<ToolSpec> = registry.specs_visible_to(permissions);
            let req = bean_llm::ChatRequest {
                system: &system,
                messages: &messages,
                tools: &tool_specs,
                max_tokens: config.llm.max_tokens,
            };
            let mut stream = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                stream = llm.chat_stream_with_model(req, &config.llm.model) => stream?,
            };
            // Reset ngay khi bắt đầu từng lượt LLM, kể cả response chỉ gọi tool và không có text.
            io.on_text_delta("", 0, true);
            let mut streamed = StreamResponseBuilder {
                text_delta_index: 1,
                emitted_text: true,
                ..StreamResponseBuilder::default()
            };
            loop {
                let item = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(AgentError::Cancelled),
                    item = stream.next() => item,
                };
                let Some(item) = item else { break };
                streamed.push(item?, io.as_ref());
            }
            let resp = streamed.finish()?;

            // Ghi vào cả sổ tổng lẫn sổ riêng role (M21.7).
            record_usage(store, &usage_day, resp.usage).await?;
            let role_usage =
                record_usage_for_role(store, &usage_day, permissions.usage_scope(), resp.usage)
                    .await?;
            let used = u64::from(role_usage.total());
            if used > role_budget {
                return finish_with_notice(
                    store,
                    session,
                    state.transcript,
                    budget_notice(used, role_budget),
                    EndReason::BudgetExceeded,
                    state.tool_call_count,
                    state.loaded_skills,
                )
                .await;
            }
            let is_final = resp.tool_calls.is_empty();
            let final_text = resp.text.clone().unwrap_or_default();

            // Provider kết thúc lượt mà **không trả về chữ nào**. Trước đây `unwrap_or_default()`
            // biến nó thành chuỗi rỗng: UI hiện một dòng assistant rỗng, người dùng thấy màn
            // hình trống và không có cách nào đoán lý do. Nguyên nhân điển hình là model
            // reasoning dùng hết `llm.max_tokens` cho `reasoning` rồi `content: null`
            // (đo thật: `nvidia/nemotron-3-super-120b-a12b:free` với `max_tokens = 2048` trả
            // `finish_reason: length` + `content: null`; nâng lên 16384 thì trả lời bình thường).
            //
            // Không ghi message rỗng vào DB: thay bằng thông báo nêu đúng nguyên nhân, để vừa
            // cho người đọc biết, vừa cho lượt sau của model biết phải làm gì.
            if is_final && final_text.trim().is_empty() {
                return finish_with_notice(
                    store,
                    session,
                    state.transcript,
                    empty_response_notice(resp.stop),
                    EndReason::EmptyResponse,
                    state.tool_call_count,
                    state.loaded_skills,
                )
                .await;
            }

            let assistant_message = Message::from_response(&resp);
            let message_id =
                append_run_message(store, session, &mut state.transcript, assistant_message)
                    .await?;

            if is_final {
                return Ok(RunOutcome {
                    text: final_text,
                    message_id: Some(message_id),
                    ended: EndReason::Final,
                    tool_call_count: state.tool_call_count,
                    loaded_skills: state.loaded_skills.into_iter().collect(),
                    transcript: state.transcript,
                });
            }
            // Text "suy nghĩ" của model khi vẫn còn gọi tool đã được phát theo từng delta
            // trong vòng stream; không phát lại toàn bộ để tránh UI nhân đôi văn bản.

            state.repeated_tool = None;
            for call in resp.tool_calls {
                state.tool_call_count = state.tool_call_count.saturating_add(1);
                self.run_tool_call(&turn, &mut state, session_policy, &call)
                    .await?;
            }
            if cancel.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            if let Some(tool) = state.repeated_tool.take() {
                return Err(AgentError::RepeatFailure(tool));
            }
        }

        let text = format!(
            "Đã dừng sau {} bước: đã đạt giới hạn số bước. Hãy nói tiếp nếu muốn tôi tiếp tục.",
            config.agent.max_steps
        );
        finish_with_notice(
            store,
            session,
            state.transcript,
            text,
            EndReason::MaxSteps,
            state.tool_call_count,
            state.loaded_skills,
        )
        .await
    }
    /// Chạy **một** tool call: chặn RBAC ở tầng thực thi, xin xác nhận theo policy,
    /// thực thi (timeout), cắt output, ghi transcript + audit và cập nhật [`TurnState`].
    ///
    /// Rút khỏi [`Agent::run`] để vòng lặp chỉ còn phần điều phối. Nhánh "bỏ qua lời
    /// gọi này" từng là `continue` trong thân cũ, nay là `return Ok(())` — cùng ngữ nghĩa
    /// vì caller vẫn xử lý các tool call còn lại của lượt.
    ///
    /// # Errors
    /// [`AgentError`] chỉ khi ghi transcript vào store lỗi; lỗi của bản thân tool được
    /// hoá thành tool result `is_error` (agents.md mục 6) chứ không `Err`.
    async fn run_tool_call(
        &self,
        turn: &Turn<'_>,
        state: &mut TurnState,
        session_policy: &SessionPolicy,
        call: &ToolCall,
    ) -> Result<(), AgentError> {
        let store = self.store;
        let registry = self.registry;
        let config = self.config;
        let audit_log = self.audit.clone();
        let alerts = self.alerts.clone();
        let session = turn.session;
        let io = &turn.io;
        let cancel = &turn.cancel;
        let channel = turn.channel;
        let permissions = turn.permissions;
        let project = turn.project;

        // (M21.5) **Chốt chặn thứ hai ở tầng thực thi.** Lọc ở `specs_visible_to` đã ngăn
        // model *thấy* tool ngoài quyền, nhưng `args` là JSON không tin cậy: nội dung
        // untrusted (mục 15.4) hoặc model bị ảo giác vẫn có thể bịa ra tên tool. Dùng
        // **cùng** `RolePermissions` và **cùng** hàm `allows` ⇒ không thể lệch nhau
        // giữa lúc lọc payload và lúc chạy, và không có logic RBAC thứ hai rải rác.
        if !registry.allows(&call.name, permissions) {
            let message = format!(
                "Bạn không có quyền (`{}`) gọi tool `{}`.",
                permissions.role, call.name
            );
            tracing::warn!(
                tool = %call.name,
                role = %permissions.role,
                "từ chối tool call ngoài quyền của role"
            );
            append_run_message(
                store,
                session,
                &mut state.transcript,
                Message::tool_error(call.id.clone(), message.clone()),
            )
            .await?;
            io.on_tool_start(&call.id, &call.name, Risk::Dangerous, &call.name, "");
            io.on_tool_end(&call.id, &call.name, false, &message);
            return Ok(());
        }

        let risk = registry
            .get(&call.name)
            .map_or(bean_types::Risk::Safe, |tool| tool.risk(&call.args));
        let args_preview = args_preview(&call.args);
        io.on_tool_start(&call.id, &call.name, risk, &call.name, &args_preview);

        if cancel.is_cancelled() {
            append_run_message(
                store,
                session,
                &mut state.transcript,
                Message::tool_error(call.id.clone(), CANCELLED_MSG),
            )
            .await?;
            io.on_tool_end(&call.id, &call.name, false, CANCELLED_MSG);
            return Ok(());
        }

        // (M21.1) Workspace của **project profile** của lượt này; rơi về workspace
        // chung khi project không có thư mục riêng. Nhờ vậy `MEMORY.md`/`USER.md` và
        // mọi thao tác file của hai project không lẫn nhau.
        let workspace = match registry.workspace_for(project) {
            Some(workspace) => workspace,
            None => {
                let message = format!("registry thiếu workspace cho project `{project}`");
                append_run_message(
                    store,
                    session,
                    &mut state.transcript,
                    Message::tool_error(call.id.clone(), message.clone()),
                )
                .await?;
                io.on_tool_end(&call.id, &call.name, false, &message);
                return Ok(());
            }
        };
        let ctx = ToolCtx {
            workspace,
            session,
            cancel: io.cancel_token().clone(),
            untrusted_seen: state.untrusted_seen.clone(),
            project: project.to_string(),
            alerts: alerts.clone(),
        };

        let args_hash = hash_args(&call.args);
        let key = (call.name.clone(), args_hash);
        if state.failure_counts.get(&key).copied().unwrap_or(0) >= 2
            && state.consecutive_same_failure.as_ref() == Some(&key)
        {
            let hint = format!(
                "Tool `{}` với cùng tham số đã thất bại 2 lần liên tiếp. Hãy thử cách khác.",
                call.name
            );
            append_run_message(
                store,
                session,
                &mut state.transcript,
                Message::tool_error(call.id.clone(), hint.clone()),
            )
            .await?;
            io.on_tool_end(&call.id, &call.name, false, &hint);
            state.repeated_tool = Some(call.name.clone());
            return Ok(());
        }

        let untrusted = state.untrusted_seen.load(Ordering::SeqCst);
        let decision = decide(&call.name, risk, &call.args, untrusted, session_policy);

        // Kết quả ghi audit cho lời gọi này (mục 15.8) — điền dần rồi ghi DUY NHẤT
        // một lần ở cuối khối (trừ nhánh deny `continue` — ghi ngay trong nhánh).
        let mut audit = entry_now(session.get(), channel, &call.name, &call.args);
        match decision {
            PolicyDecision::Allowed => {
                audit.decision = "allow";
                audit.decided_by = "policy".into();
            }
            PolicyDecision::NeedsConfirm { allow_in_session } => {
                if cancel.is_cancelled() {
                    append_run_message(
                        store,
                        session,
                        &mut state.transcript,
                        Message::tool_error(call.id.clone(), CANCELLED_MSG),
                    )
                    .await?;
                    io.on_tool_end(&call.id, &call.name, false, CANCELLED_MSG);
                    audit.decision = "deny";
                    audit.decided_by = "cancelled".into();
                    record_audit(audit_log.as_ref(), &audit);
                    return Ok(());
                }
                let deny_note = deny_list_reason(&call.name, &call.args)
                    .map(|reason| format!("\n[cảnh báo deny-list] {}", reason.label))
                    .unwrap_or_default();
                let prompt = format!("{} {}{deny_note}", call.name, args_preview);
                let replied = if io.is_background() && untrusted {
                    Some(Decision::Deny)
                } else {
                    io.confirm(
                        &call.id,
                        &call.name,
                        risk,
                        &prompt,
                        allow_in_session,
                        CONFIRM_TIMEOUT,
                    )
                    .await
                };
                let actor = io.decision_actor();
                match replied {
                    Some(Decision::Allow) => {
                        audit.decision = "allow";
                        audit.decided_by = actor.unwrap_or_else(|| "user".into());
                    }
                    Some(Decision::AllowInSession) if allow_in_session => {
                        session_policy.allow(&call.name);
                        audit.decision = "allow_in_session";
                        audit.decided_by = actor.unwrap_or_else(|| "user".into());
                    }
                    Some(Decision::AllowInSession) | Some(Decision::Deny) => {
                        audit.decision = "deny";
                        audit.decided_by = actor.unwrap_or_else(|| "user".into());
                        record_audit(audit_log.as_ref(), &audit);
                        let message = "Người dùng đã từ chối hành động này.".to_string();
                        append_run_message(
                            store,
                            session,
                            &mut state.transcript,
                            Message::tool_error(call.id.clone(), message.clone()),
                        )
                        .await?;
                        io.on_tool_end(&call.id, &call.name, false, &message);
                        return Ok(());
                    }
                    None => {
                        let message = if cancel.is_cancelled() {
                            audit.decided_by = "cancelled".into();
                            CANCELLED_MSG.to_string()
                        } else {
                            audit.decided_by = "timeout".into();
                            "Hết thời gian chờ xác nhận — hành động bị từ chối.".to_string()
                        };
                        audit.decision = "deny";
                        record_audit(audit_log.as_ref(), &audit);
                        append_run_message(
                            store,
                            session,
                            &mut state.transcript,
                            Message::tool_error(call.id.clone(), message.clone()),
                        )
                        .await?;
                        io.on_tool_end(&call.id, &call.name, false, &message);
                        return Ok(());
                    }
                }
            }
        }

        // Chạy tool, đua với huỷ (mục 6) và timeout. Khi token bị huỷ giữa chừng:
        // ghi tool result "[bị người dùng huỷ]" để lịch sử giữ cặp tool_use/tool_result
        // hợp lệ rồi kết thúc run. Ghi DB diễn ra SAU khi select hoàn tất nên không
        // bị cắt giữa lúc ghi (cancel-safety của `select!`, mục 22.10).
        // (M26) `call_rich` cho phép tool trả ảnh; `call` cũ vẫn chạy qua default
        // implementation nên **mọi tool cũ hành xử y hệt**.
        enum ExecOutcome {
            Done(Result<ToolOutput, ToolError>),
            Timeout,
            Cancelled,
        }
        let exec = timeout(
            Duration::from_secs(config.security.tool_timeout_seconds),
            execute_tool(registry, &ctx, call),
        );
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => ExecOutcome::Cancelled,
            r = exec => match r {
                Ok(inner) => ExecOutcome::Done(inner),
                Err(_) => ExecOutcome::Timeout,
            },
        };

        let (ok, output, image_block, cancelled) = match outcome {
            ExecOutcome::Done(Ok(ToolOutput::Image { caption, image })) => {
                (true, caption, Some(image), false)
            }
            ExecOutcome::Done(Ok(ToolOutput::Text(text))) => (true, text, None, false),
            ExecOutcome::Done(Err(e)) => (
                false,
                format!("Lỗi tool `{}`: {}", call.name, e),
                None,
                false,
            ),
            ExecOutcome::Timeout => (
                false,
                format!(
                    "Tool `{}` hết thời gian cho phép ({} giây).",
                    call.name, config.security.tool_timeout_seconds
                ),
                None,
                false,
            ),
            ExecOutcome::Cancelled => (false, CANCELLED_MSG.to_string(), None, true),
        };

        // Ảnh KHÔNG đi qua `truncate_output`: cắt theo ký tự một chuỗi base64 sẽ
        // sinh PNG hỏng. Trần byte đã do tool áp (`[browser].max_image_bytes`).
        let output = truncate_output(&output);
        // Audit **không** bao giờ ghi base64 (mục 15.8): chỉ ghi tham chiếu.
        if let Some(image) = image_block.as_ref() {
            audit.artifact = Some(format!(
                "image:{}:{}:{} bytes",
                image.media_type,
                image.sha256,
                image.data.len()
            ));
        }
        // (M4, mục 15.4) Bật cờ untrusted cho cả lượt khi tool trả nội dung ngoài lõi:
        // mọi confirm Confirm/Dangerous SAU đây sẽ hỏi lại, không "trong phiên".
        //
        // Có hai lớp, cùng dùng để không lệ thuộc vào một quy ước ngầm:
        // 1. `Tool::marks_untrusted()` — khai báo tường minh của tool. Đây là lớp
        //    chính: tool mới quên bọc sẽ bị test hồi quy bắt, không hỏng âm thầm.
        // 2. `contains_untrusted_block` — lưới an toàn cho output thực sự mang thẻ
        //    (kể cả tool tự bọc tay như `web_fetch`, hoặc lỗi từ MCP đã bọc sẵn).
        let marks_untrusted = registry
            .get(&call.name)
            .is_some_and(|tool| tool.marks_untrusted());
        if marks_untrusted || contains_untrusted_block(&output) {
            state.untrusted_seen.store(true, Ordering::SeqCst);
        }
        // Audit kết quả thực thi (mục 15.8) — lỗi ghi chỉ là cảnh báo, không làm hỏng run.
        audit.ok = Some(ok);
        if !ok {
            audit.error = Some(output.chars().take(300).collect());
        }
        record_audit(audit_log.as_ref(), &audit);
        let mut failed_twice = false;
        if !ok {
            let count = state.failure_counts.entry(key.clone()).or_insert(0);
            *count = count.saturating_add(1);
            failed_twice = *count >= 2;
            state.consecutive_same_failure = Some(key);
        } else {
            state.consecutive_same_failure = None;
        }
        let output = if failed_twice {
            // Chèn gợi ý ngay trong tool result sau 2 lần thất bại giống nhau (mục 6).
            format!("{output}\n[Gợi ý] Hai lần gọi giống nhau đều thất bại — hãy thử cách khác.")
        } else {
            output
        };
        // (M26) Tool trả **ảnh** dùng `Message::tool_with_image`: caption vẫn
        // đi qua đường cắt ký tự như mọi tool, còn ảnh đi kèm nguyên vẹn.
        let result_message = match (ok, image_block) {
            (true, Some(image)) => Message::tool_with_image(call.id.clone(), output.clone(), image),
            (true, None) => Message::tool(call.id.clone(), output.clone()),
            (false, _) => Message::tool_error(call.id.clone(), output.clone()),
        };
        append_run_message(store, session, &mut state.transcript, result_message).await?;
        if ok
            && call.name == "load_skill"
            && let Some(name) = call.args.get("name").and_then(serde_json::Value::as_str)
        {
            state.loaded_skills.insert(name.to_string());
        }
        io.on_tool_end(&call.id, &call.name, ok, &output);
        if cancelled {
            return Ok(());
        }
        Ok(())
    }
}

async fn finish_with_notice(
    store: &dyn Store,
    session: bean_types::SessionId,
    mut transcript: Vec<Message>,
    text: String,
    ended: EndReason,
    tool_call_count: usize,
    loaded_skills: BTreeSet<String>,
) -> Result<RunOutcome, AgentError> {
    let message_id = append_run_message(
        store,
        session,
        &mut transcript,
        Message::assistant(Some(text.clone()), Vec::new()),
    )
    .await?;
    Ok(RunOutcome {
        text,
        message_id: Some(message_id),
        ended,
        tool_call_count,
        loaded_skills: loaded_skills.into_iter().collect(),
        transcript,
    })
}

fn budget_notice(used: u64, limit: u64) -> String {
    format!(
        "Đã dừng: ngân sách token/ngày đã đạt {used}/{limit} token. Hãy tiếp tục vào ngày mới hoặc tăng `security.daily_token_budget`."
    )
}

/// Thông báo khi provider kết thúc lượt mà không trả về chữ nào.
///
/// Tách nhánh `MaxTokens` vì đó là nguyên nhân đo được và **hành động sửa được**:
/// model reasoning dùng hết ngân sách token cho phần `reasoning` nên `content` về
/// `null`. Nói rõ cách sửa tốt hơn nhiều so với trả chuỗi rỗng.
fn empty_response_notice(stop: StopReason) -> String {
    match stop {
        StopReason::MaxTokens => {
            "Model đã dùng hết `llm.max_tokens` cho phần suy luận nội bộ và chưa kịp \
             trả lời (provider báo `finish_reason: length`, nội dung rỗng). Hãy tăng \
             `llm.max_tokens` trong `bean.toml`, hoặc đổi sang model không suy luận dài. \
             Yêu cầu của bạn chưa được xử lý — hãy gửi lại sau khi đổi cấu hình."
                .to_string()
        }
        _ => "Model kết thúc lượt mà không trả về nội dung nào (không phải lỗi xác thực \
             hay hết ngân sách). Yêu cầu của bạn chưa được xử lý — hãy thử lại, hoặc đổi \
             model trong `bean.toml` nếu tình trạng này lặp lại."
            .to_string(),
    }
}

async fn append_run_message(
    store: &dyn Store,
    session: bean_types::SessionId,
    transcript: &mut Vec<Message>,
    message: Message,
) -> Result<i64, crate::store::StoreError> {
    let id = store.append(session, message.clone()).await?;
    transcript.push(message);
    Ok(id)
}

/// Ghi một bản ghi audit — lỗi được log cảnh báo và **không** làm hỏng vòng lặp
/// (audit là observability, không phải rào cản an ninh — mục 15.8).
fn record_audit(log: Option<&Arc<AuditLog>>, entry: &AuditEntry) {
    if let Some(log) = log
        && let Err(err) = log.record(entry)
    {
        tracing::warn!("ghi audit log thất bại: {err}");
    }
}

async fn execute_tool(
    registry: &bean_tools::ToolRegistry,
    ctx: &ToolCtx,
    call: &ToolCall,
) -> Result<ToolOutput, ToolError> {
    let tool = registry
        .get(&call.name)
        .ok_or_else(|| ToolError::NotFound(call.name.clone()))?;
    // (M26) `call_rich` mặc định gọi `call`, nên tool cũ không đổi hành vi.
    tool.call_rich(ctx, call.args.clone()).await
}

fn truncate_output(output: &str) -> String {
    let len = output.chars().count();
    if len <= MAX_TOOL_OUTPUT_CHARS {
        return output.to_string();
    }
    let cut_pos = output
        .char_indices()
        .enumerate()
        .find_map(|(count, (idx, _))| {
            if count >= MAX_TOOL_OUTPUT_CHARS {
                Some(idx)
            } else {
                None
            }
        })
        .unwrap_or(output.len());
    let mut result = output[..cut_pos].to_string();
    result.push_str(&format!(
        "[đã cắt {} ký tự, dùng offset để đọc tiếp]",
        len - MAX_TOOL_OUTPUT_CHARS
    ));
    result
}

fn args_preview(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => {
            let mut parts = Vec::new();
            for (k, v) in map.iter() {
                let v_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                parts.push(format!("{}={}", k, v_str));
            }
            parts.join("; ")
        }
        _ => args.to_string(),
    }
}

fn hash_args(args: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut s = DefaultHasher::new();
    args.hash(&mut s);
    format!("{:#x}", s.finish())
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("lỗi lưu trữ: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("lỗi provider (LLM): {0}")]
    Llm(String),
    /// Người dùng huỷ run (`CancellationToken` tường minh — mục 10).
    #[error("run đã bị huỷ")]
    Cancelled,
    #[error("tool `{}' thất bại liên tiếp — hãy thử cách khác", 0)]
    RepeatFailure(String),
}

impl AgentError {
    /// Mã lỗi ổn định cho adapter, không chứa nội dung bí mật.
    ///
    /// Đặt cạnh [`RouterError`](crate::router::RouterError) như một **method** để
    /// biểu đồ "loại lỗi ⇒ mã ổn định" nằm ngay trên kiểu lỗi, không phải ở
    /// module khác — trước đây có hai bản `match` trùng nhau dễ lệch khi thêm biến thể.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Store(_) => "store",
            Self::Llm(_) => "llm",
            Self::Cancelled => "cancelled",
            Self::RepeatFailure(_) => "loop_guard",
        }
    }
}

impl From<LlmError> for AgentError {
    fn from(err: LlmError) -> Self {
        AgentError::Llm(err.to_string())
    }
}

#[cfg(test)]
mod truncate_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{MAX_TOOL_OUTPUT_CHARS, truncate_output};

    #[test]
    fn truncate_respects_utf8_boundaries() {
        // Tiếng Việt + emoji (ký tự đa byte) — cắt không được panic và không cắt giữa ký tự.
        let vi = "Xin chào thế giới 🦀 — dấu ệ ư ơ đ 🎏".repeat(1500);
        let out = truncate_output(&vi);
        assert!(out.contains("đã cắt"));
        assert!(out.chars().count() <= MAX_TOOL_OUTPUT_CHARS + 80);

        // Dưới ngưỡng: nguyên vẹn.
        assert_eq!(truncate_output("ngắn 🦀"), "ngắn 🦀");

        // Chuỗi toàn emoji 4 byte: phải cắt đúng tại ranh giới ký tự.
        let emoji = "🦀".repeat(MAX_TOOL_OUTPUT_CHARS + 7);
        let out = truncate_output(&emoji);
        assert!(out.chars().count() >= MAX_TOOL_OUTPUT_CHARS);
        // Phần thân (trước ghi chú) phải parse được thành String hợp lệ (không panic).
        let body: String = out.split("[đã cắt").next().unwrap().to_string();
        assert_eq!(body.chars().count(), MAX_TOOL_OUTPUT_CHARS);
    }
}
