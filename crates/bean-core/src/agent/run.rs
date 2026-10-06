//! Vòng lặp chính: [`Agent::run`] gom text/tool-call từng bước, chống lặp,
//! giới hạn ngân sách và kết thúc lượt bằng thông báo (`finish_with_notice`).
//! Kèm [`StreamResponseBuilder`] lắp ráp deltas khi provider streaming.

use super::*;

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
