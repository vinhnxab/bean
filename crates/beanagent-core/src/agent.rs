//! Vòng lặp agent (agents.md mục 6).

use std::collections::HashMap;
use std::time::Duration;

use crate::store::Store;
use beanagent_llm::{LlmError, LlmProvider};
use beanagent_tools::{ToolCtx, ToolError};
use beanagent_types::{Config, Message, Role, ToolCall, ToolSpec};
use tokio::time::timeout;

use crate::run_io::{Decision, RunIo};

const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;
const CANCELLED_MSG: &str = "[bị người dùng huỷ]";
type RepeatKey = (String, String);

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
    pub io: &'a dyn RunIo,
    /// Token huỷ run đang chạy.
    pub cancel: tokio_util::sync::CancellationToken,
}

pub async fn run_turn(args: RunTurnArgs<'_>) -> Result<String, AgentError> {
    let RunTurnArgs {
        store,
        registry,
        llm,
        config,
        session,
        user_text,
        io,
        cancel,
    } = args;
    store.append(session, Message::user(user_text)).await?;
    let mut failure_counts: HashMap<RepeatKey, u32> = HashMap::new();
    let mut consecutive_same_failure: Option<RepeatKey> = None;

    for _step in 0..config.agent.max_steps {
        let system = crate::prompt::system_prompt(&config.agent, "", "", "");
        let history = store.history(session, None, 200).await?;
        let messages: Vec<Message> = system_to_messages(&system)
            .into_iter()
            .chain(history)
            .collect();

        let tool_specs: Vec<ToolSpec> = registry.specs();
        let req = beanagent_llm::ChatRequest {
            system: &system,
            messages: &messages,
            tools: &tool_specs,
            max_tokens: config.llm.max_tokens,
        };
        let resp = llm.chat(req).await?;

        store.append(session, Message::from_response(&resp)).await?;

        if resp.tool_calls.is_empty() {
            // Response cuối: trả về để kênh hiển thị (CLI/WS in một lần duy nhất,
            // không phát `on_text` nữa để tránh nhân đôi).
            return Ok(resp.text.unwrap_or_default());
        }
        // Text "suy nghĩ" của model khi vẫn còn gọi tool — phát cho kênh hiển thị.
        io.on_text(&resp.text.clone().unwrap_or_default());

        for call in resp.tool_calls {
            let workspace = registry.workspace().map_err(|e| {
                let msg = format!("registry thiếu workspace: {e}");
                // Ghi tool result lỗi để lịch sử không hỏng cặp, rồi dừng lượt này.
                msg
            });
            let workspace = match workspace {
                Ok(ws) => ws,
                Err(msg) => {
                    store
                        .append(session, Message::tool_error(call.id, msg.clone()))
                        .await?;
                    io.on_tool_end(&call.name, false, &msg);
                    continue;
                }
            };
            let ctx = ToolCtx {
                workspace,
                session,
                cancel: io.cancel_token().clone(),
                untrusted_seen: Default::default(),
            };

            let args_hash = hash_args(&call.args);
            let key = (call.name.clone(), args_hash);
            // Chống lặp (mục 6): cùng tool + cùng tham số đã thất bại 2 lần liên tiếp →
            // lần gọi thứ 3 bị chặn, dừng run để model không quay mãi vô hạn.
            if failure_counts.get(&key).copied().unwrap_or(0) >= 2
                && consecutive_same_failure.as_ref() == Some(&key)
            {
                let hint = format!(
                    "Tool `{}` với cùng tham số đã thất bại 2 lần liên tiếp. Hãy thử cách khác.",
                    call.name
                );
                store
                    .append(session, Message::tool_error(call.id, hint.clone()))
                    .await?;
                io.on_tool_end(&call.name, false, &hint);
                return Err(AgentError::RepeatFailure(call.name));
            }

            io.on_tool_start(&call.name, &call.name, &args_preview(&call.args));

            // Xác nhận theo mức rủi ro (mục 7.2). Confirm/Dangerous phải được người dùng
            // cho phép; từ chối biến thành tool result is_error để model tự điều chỉnh.
            let risk = registry
                .get(&call.name)
                .map_or(beanagent_types::Risk::Safe, |t| t.risk(&call.args));
            if risk != beanagent_types::Risk::Safe {
                if cancel.is_cancelled() {
                    store
                        .append(session, Message::tool_error(&call.id, CANCELLED_MSG))
                        .await?;
                    io.on_tool_end(&call.name, false, CANCELLED_MSG);
                    return Err(AgentError::Cancelled);
                }
                let prompt = format!("{} {}", call.name, args_preview(&call.args));
                match io
                    .confirm(
                        &prompt,
                        risk == beanagent_types::Risk::Confirm,
                        Duration::from_secs(300),
                    )
                    .await
                {
                    Some(Decision::Allow) | Some(Decision::AllowInSession) => {}
                    _ => {
                        let msg = "Người dùng đã từ chối hành động này.".to_string();
                        store
                            .append(session, Message::tool_error(&call.id, msg.clone()))
                            .await?;
                        io.on_tool_end(&call.name, false, &msg);
                        continue;
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
            store
                .append(
                    session,
                    if ok {
                        Message::tool(&call.id, output.clone())
                    } else {
                        Message::tool_error(&call.id, output.clone())
                    },
                )
                .await?;
            io.on_tool_end(&call.name, ok, &output);
            if cancelled {
                return Err(AgentError::Cancelled);
            }
            if cancelled {
                return Err(AgentError::Cancelled);
            }
        }
    }

    Ok("Đã đạt giới hạn số bước. Hãy nói tiếp nếu muốn tôi tiếp tục.".into())
}

fn system_to_messages(system: &str) -> Vec<Message> {
    vec![Message {
        role: Role::User,
        text: Some(system.to_string()),
        tool_calls: Vec::new(),
        tool_call_id: None,
        is_error: false,
    }]
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
