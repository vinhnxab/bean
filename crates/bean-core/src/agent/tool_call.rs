//! Thực thi một tool call: xác định rủi ro, xin xác nhận, chạy, cắt output,
//! ghi audit và trả về `Message` kết quả (`Agent::run_tool_call`).

use super::*;

impl<'a> Agent<'a> {
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
    pub(super) async fn run_tool_call(
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

#[cfg(test)]
mod truncate_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::MAX_TOOL_OUTPUT_CHARS;
    use super::truncate_output;

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
