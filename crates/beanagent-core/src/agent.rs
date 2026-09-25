//! Vòng lặp agent (agents.md mục 6, 7.2, 15.3, 15.4, 15.8).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::store::{Store, StoreError};
use beanagent_llm::{LlmError, LlmProvider};
use beanagent_memory::{ensure_daily_budget, record_usage};
use beanagent_security::audit::{AuditEntry, AuditLog, entry_now};
use beanagent_security::policy::{Policy, PolicyDecision, SessionPolicy, deny_list_reason};
use beanagent_security::untrusted::contains_untrusted_block;
use beanagent_tools::{ToolCtx, ToolError};
use beanagent_types::{
    Config, LlmDelta, LlmResponse, Message, StopReason, ToolCall, ToolSpec, Usage,
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

/// Tham số cho [`run_turn`]: gộp lại thành struct để tránh quá nhiều tham số hàm
/// (clippy `too_many_arguments`).
pub struct RunTurnArgs<'a> {
    /// Store lịch sử hội thoại.
    pub store: &'a dyn Store,
    /// Registry tool đã đăng ký.
    pub registry: &'a beanagent_tools::ToolRegistry,
    /// Provider LLM.
    pub llm: &'a dyn LlmProvider,
    /// Cấu hình chung.
    pub config: &'a Config,
    /// Phiên hội thoại.
    pub session: beanagent_types::SessionId,
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
        audit: audit_log,
        channel,
        skills_index,
    } = args;
    // (M5/D8.10) System prompt chỉ đi qua `ChatRequest.system` — **không** nhân bản nó
    // thành message `User` (M3 từng làm vậy: tốn token gấp đôi cho phần system và dễ
    // bị model hiểu nhầm là câu lệnh của người dùng). Giữ `turn_input` để dựng lỡ
    // trường hợp lịch sử rỗng.
    let turn_input = user_text;
    let mut transcript = Vec::new();
    let mut tool_call_count = 0_usize;
    let mut loaded_skills = BTreeSet::new();
    append_run_message(
        store,
        session,
        &mut transcript,
        Message::user(turn_input.clone()),
    )
    .await?;
    match store.compact(session, llm, config).await {
        Ok(()) => {}
        Err(StoreError::BudgetExceeded { used, limit }) => {
            return finish_with_notice(
                store,
                session,
                transcript,
                budget_notice(used, limit),
                EndReason::BudgetExceeded,
                tool_call_count,
                loaded_skills,
            )
            .await;
        }
        Err(error) => return Err(error.into()),
    }
    let mut failure_counts: HashMap<RepeatKey, u32> = HashMap::new();
    let mut consecutive_same_failure: Option<RepeatKey> = None;
    // (M4, mục 15.4) Cờ untrusted dùng chung cho MỌI tool trong lượt — khi một tool
    // result chứa khối <untrusted_content>, mọi tool Confirm trở lên phải hỏi lại.
    let untrusted_seen = Arc::new(AtomicBool::new(false));
    // Session policy dùng chung; không truyền vào thì mỗi lượt hỏi lại (an toàn mặc định).
    let local_session_policy;
    let session_policy: &SessionPolicy = match &session_policy {
        Some(p) => p.as_ref(),
        None => {
            local_session_policy = SessionPolicy::new();
            &local_session_policy
        }
    };
    let policy = Policy::new();

    for _step in 0..config.agent.max_steps {
        // (M5, mục 8.2) Dựng context: system prompt + MEMORY.md/USER.md + summary của phiên
        // + lịch sử vừa ngân sách token, cắt ở ranh giới an toàn (không tách cặp tool).
        let workspace = registry.workspace_opt();
        let ctx = crate::context::build_with_skills(
            store,
            config,
            session,
            workspace.as_deref(),
            skills_index,
        )
        .await?;
        let system = ctx.system;
        let mut messages = ctx.messages;
        if messages.is_empty() {
            // Provider (Anthropic) từ chối `messages: []`. Sau `append` ở trên lịch sử
            // không bao giờ rỗng — đây chỉ là lưới an toàn, không phải đường đi bình thường.
            messages.push(Message::user(turn_input.clone()));
        }

        let usage_day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        match ensure_daily_budget(store, &usage_day, config.security.daily_token_budget).await {
            Ok(()) => {}
            Err(StoreError::BudgetExceeded { used, limit }) => {
                return finish_with_notice(
                    store,
                    session,
                    transcript,
                    budget_notice(used, limit),
                    EndReason::BudgetExceeded,
                    tool_call_count,
                    loaded_skills,
                )
                .await;
            }
            Err(error) => return Err(error.into()),
        }
        let tool_specs: Vec<ToolSpec> = registry.specs();
        let req = beanagent_llm::ChatRequest {
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

        let usage = record_usage(store, &usage_day, resp.usage).await?;
        let used = u64::from(usage.total());
        if used > config.security.daily_token_budget {
            return finish_with_notice(
                store,
                session,
                transcript,
                budget_notice(used, config.security.daily_token_budget),
                EndReason::BudgetExceeded,
                tool_call_count,
                loaded_skills,
            )
            .await;
        }
        let is_final = resp.tool_calls.is_empty();
        let final_text = resp.text.clone().unwrap_or_default();
        let assistant_message = Message::from_response(&resp);
        let message_id =
            append_run_message(store, session, &mut transcript, assistant_message).await?;

        if is_final {
            return Ok(RunOutcome {
                text: final_text,
                message_id: Some(message_id),
                ended: EndReason::Final,
                tool_call_count,
                loaded_skills: loaded_skills.into_iter().collect(),
                transcript,
            });
        }
        // Text "suy nghĩ" của model khi vẫn còn gọi tool đã được phát theo từng delta
        // trong vòng stream; không phát lại toàn bộ để tránh UI nhân đôi văn bản.

        let mut repeated_tool: Option<String> = None;

        for call in resp.tool_calls {
            tool_call_count = tool_call_count.saturating_add(1);
            let risk = registry
                .get(&call.name)
                .map_or(beanagent_types::Risk::Safe, |tool| tool.risk(&call.args));
            let args_preview = args_preview(&call.args);
            io.on_tool_start(&call.id, &call.name, risk, &call.name, &args_preview);

            if cancel.is_cancelled() {
                append_run_message(
                    store,
                    session,
                    &mut transcript,
                    Message::tool_error(call.id.clone(), CANCELLED_MSG),
                )
                .await?;
                io.on_tool_end(&call.id, &call.name, false, CANCELLED_MSG);
                continue;
            }

            let workspace = match registry.workspace() {
                Ok(workspace) => workspace,
                Err(error) => {
                    let message = format!("registry thiếu workspace: {error}");
                    append_run_message(
                        store,
                        session,
                        &mut transcript,
                        Message::tool_error(call.id.clone(), message.clone()),
                    )
                    .await?;
                    io.on_tool_end(&call.id, &call.name, false, &message);
                    continue;
                }
            };
            let ctx = ToolCtx {
                workspace,
                session,
                cancel: io.cancel_token().clone(),
                untrusted_seen: untrusted_seen.clone(),
            };

            let args_hash = hash_args(&call.args);
            let key = (call.name.clone(), args_hash);
            if failure_counts.get(&key).copied().unwrap_or(0) >= 2
                && consecutive_same_failure.as_ref() == Some(&key)
            {
                let hint = format!(
                    "Tool `{}` với cùng tham số đã thất bại 2 lần liên tiếp. Hãy thử cách khác.",
                    call.name
                );
                append_run_message(
                    store,
                    session,
                    &mut transcript,
                    Message::tool_error(call.id.clone(), hint.clone()),
                )
                .await?;
                io.on_tool_end(&call.id, &call.name, false, &hint);
                repeated_tool = Some(call.name);
                continue;
            }

            let untrusted = untrusted_seen.load(Ordering::SeqCst);
            let decision = policy.decide(&call.name, risk, &call.args, untrusted, session_policy);

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
                            &mut transcript,
                            Message::tool_error(call.id.clone(), CANCELLED_MSG),
                        )
                        .await?;
                        io.on_tool_end(&call.id, &call.name, false, CANCELLED_MSG);
                        audit.decision = "deny";
                        audit.decided_by = "cancelled".into();
                        record_audit(audit_log.as_ref(), &audit);
                        continue;
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
                                &mut transcript,
                                Message::tool_error(call.id.clone(), message.clone()),
                            )
                            .await?;
                            io.on_tool_end(&call.id, &call.name, false, &message);
                            continue;
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
                                &mut transcript,
                                Message::tool_error(call.id.clone(), message.clone()),
                            )
                            .await?;
                            io.on_tool_end(&call.id, &call.name, false, &message);
                            continue;
                        }
                    }
                }
            }

            // Chạy tool, đua với huỷ (mục 6) và timeout. Khi token bị huỷ giữa chừng:
            // ghi tool result "[bị người dùng huỷ]" để lịch sử giữ cặp tool_use/tool_result
            // hợp lệ rồi kết thúc run. Ghi DB diễn ra SAU khi select hoàn tất nên không
            // bị cắt giữa lúc ghi (cancel-safety của `select!`, mục 22.10).
            enum ExecOutcome {
                Done(Result<String, ToolError>),
                Timeout,
                Cancelled,
            }
            let exec = timeout(
                Duration::from_secs(config.security.tool_timeout_seconds),
                execute_tool(registry, &ctx, &call),
            );
            let outcome = tokio::select! {
                biased;
                _ = cancel.cancelled() => ExecOutcome::Cancelled,
                r = exec => match r {
                    Ok(inner) => ExecOutcome::Done(inner),
                    Err(_) => ExecOutcome::Timeout,
                },
            };

            let (ok, output, cancelled) = match outcome {
                ExecOutcome::Done(Ok(out)) => (true, out, false),
                ExecOutcome::Done(Err(e)) => {
                    (false, format!("Lỗi tool `{}`: {}", call.name, e), false)
                }
                ExecOutcome::Timeout => (
                    false,
                    format!(
                        "Tool `{}` hết thời gian cho phép ({} giây).",
                        call.name, config.security.tool_timeout_seconds
                    ),
                    false,
                ),
                ExecOutcome::Cancelled => (false, CANCELLED_MSG.to_string(), true),
            };

            let output = truncate_output(&output);
            // (M4, mục 15.4) Tool result chứa khối untrusted → bật cờ cho cả lượt:
            // mọi confirm Confirm/Dangerous SAU đây sẽ hỏi lại, không "trong phiên".
            if contains_untrusted_block(&output) {
                untrusted_seen.store(true, Ordering::SeqCst);
            }
            // Audit kết quả thực thi (mục 15.8) — lỗi ghi chỉ là cảnh báo, không làm hỏng run.
            audit.ok = Some(ok);
            if !ok {
                audit.error = Some(output.chars().take(300).collect());
            }
            record_audit(audit_log.as_ref(), &audit);
            let mut failed_twice = false;
            if !ok {
                let count = failure_counts.entry(key.clone()).or_insert(0);
                *count = count.saturating_add(1);
                failed_twice = *count >= 2;
                consecutive_same_failure = Some(key);
            } else {
                consecutive_same_failure = None;
            }
            let output = if failed_twice {
                // Chèn gợi ý ngay trong tool result sau 2 lần thất bại giống nhau (mục 6).
                format!(
                    "{output}\n[Gợi ý] Hai lần gọi giống nhau đều thất bại — hãy thử cách khác."
                )
            } else {
                output
            };
            let result_message = if ok {
                Message::tool(call.id.clone(), output.clone())
            } else {
                Message::tool_error(call.id.clone(), output.clone())
            };
            append_run_message(store, session, &mut transcript, result_message).await?;
            if ok
                && call.name == "load_skill"
                && let Some(name) = call.args.get("name").and_then(serde_json::Value::as_str)
            {
                loaded_skills.insert(name.to_string());
            }
            io.on_tool_end(&call.id, &call.name, ok, &output);
            if cancelled {
                continue;
            }
        }
        if cancel.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        if let Some(tool) = repeated_tool.take() {
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
        transcript,
        text,
        EndReason::MaxSteps,
        tool_call_count,
        loaded_skills,
    )
    .await
}

async fn finish_with_notice(
    store: &dyn Store,
    session: beanagent_types::SessionId,
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

async fn append_run_message(
    store: &dyn Store,
    session: beanagent_types::SessionId,
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
    registry: &beanagent_tools::ToolRegistry,
    ctx: &ToolCtx,
    call: &ToolCall,
) -> Result<String, ToolError> {
    let tool = registry
        .get(&call.name)
        .ok_or_else(|| ToolError::NotFound(call.name.clone()))?;
    tool.call(ctx, call.args.clone()).await
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
