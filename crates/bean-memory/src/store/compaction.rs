//! Compaction: nén lịch sử cũ thành tóm tắt (agents.md mục 8.3).
//!
//! # Vì sao tách khỏi [`super`]
//!
//! Compaction là **quy trình nhiều bước**: đo ngân sách → tìm ranh giới an toàn → gọi
//! LLM một lượt → ghi summary → xoá phần cũ. Nó chạy giống nhau trên mọi bản cài đặt
//! (chỉ khác ở hai lệnh `list_messages`/`save_summary`/`delete_before` mà trait đã
//! đảm bảo), nên nó thuộc về **tầng trên store**, không thuộc về bản cài đặt nào.
//!
//! Đây cũng là nơi bất biến khó nhất của cả tầng nằm: **không bao giờ** cắt giữa một
//! cặp `assistant(tool_calls)` và các `tool` result của nó, vì API sẽ trả 400
//! (agents.md mục 22.1). Trước khi tách, bất biến này nằm lẫn giữa SQL của FTS5 và
//! hàng đợi task — không ai tìm được nơi cần sửa khi nó sai.
/// System prompt cho lượt "reflection" chỉ để nén lịch sử (agents.md mục 8.3).
use bean_llm::{ChatRequest, LlmProvider};
use bean_types::{Config, Message, SessionId};

use crate::safe_cut::{check_no_orphan_result, find_compaction_start};

use super::budget::{ensure_daily_budget, record_usage};
use super::shared::{estimate_tokens, render_transcript, truncate_chars};
use super::{
    COMPACT_KEEP_RECENT, COMPACT_TRIGGER_PERCENT, MAX_SUMMARY_INPUT_CHARS, Store, StoreError,
};
const SUMMARY_SYSTEM_PROMPT: &str = "You compress conversation history for a personal AI agent. \
Write a dense factual summary in the same language as the conversation. Keep: the current goal, \
decisions already made, important file paths and commands, and unfinished work. Never invent \
facts that are not in the transcript. Output only the summary, no preamble.";

/// Gọi LLM một lượt để tóm tắt phần lịch sử cũ.
async fn summarize(
    store: &dyn Store,
    llm: &dyn LlmProvider,
    config: &Config,
    previous: Option<&str>,
    old: &[Message],
) -> Result<String, StoreError> {
    let transcript = truncate_chars(&render_transcript(old), MAX_SUMMARY_INPUT_CHARS);
    let mut user = String::new();
    if let Some(prev) = previous {
        user.push_str("Tóm tắt đã có của phần hội thoại trước đó:\n");
        user.push_str(prev);
        user.push_str("\n\n");
    }
    user.push_str("Hội thoại cần nén (cũ → mới):\n");
    user.push_str(&transcript);
    user.push_str(
        "\nHãy viết bản tóm tắt mới gộp cả phần đã có ở trên và phần vừa nêu. Giữ: mục tiêu đang làm, \
         quyết định đã chốt, file/đường dẫn quan trọng, việc còn dang dở.",
    );

    let messages = [Message::user(user)];
    let request = ChatRequest {
        system: SUMMARY_SYSTEM_PROMPT,
        messages: &messages,
        tools: &[],
        max_tokens: config.llm.max_tokens.min(2048),
    };
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    ensure_daily_budget(store, &day, config.security.daily_token_budget).await?;
    let response = llm
        .chat_with_model(request, &config.llm.model)
        .await
        .map_err(|err| StoreError::Internal(format!("gọi LLM để tóm tắt thất bại: {err}")))?;
    let usage = record_usage(store, &day, response.usage).await?;
    let used = u64::from(usage.total());
    if used > config.security.daily_token_budget {
        return Err(StoreError::BudgetExceeded {
            used,
            limit: config.security.daily_token_budget,
        });
    }
    Ok(response.text.unwrap_or_default())
}

/// Cài đặt compaction **một lần** cho mọi `Store`: đo ngân sách, chọn ranh giới an toàn,
/// tóm tắt phần cũ rồi xoá nó.
///
/// Best-effort: mọi lỗi ở bước tóm tắt chỉ ghi log và **giữ nguyên** lịch sử — compaction
/// không bao giờ được làm hỏng run đang chạy (agents.md mục 0.8, 8.3).
pub(crate) async fn compact_via(
    store: &dyn Store,
    session: SessionId,
    llm: &dyn LlmProvider,
    config: &Config,
) -> Result<(), StoreError> {
    let budget = u64::from(config.agent.context_budget_tokens);
    let trigger = budget.saturating_mul(COMPACT_TRIGGER_PERCENT) / 100;

    let stored = store.list_messages(session, None, 0).await?;
    let messages: Vec<Message> = stored.iter().map(|row| row.message.clone()).collect();
    if estimate_tokens(&messages) <= trigger {
        return Ok(());
    }

    let Some(start) = find_compaction_start(&messages, COMPACT_KEEP_RECENT) else {
        // Không có ranh giới an toàn (hiếm): giữ nguyên còn hơn làm hỏng cặp tool.
        tracing::debug!(session = %session, "compaction: không tìm được ranh giới an toàn");
        return Ok(());
    };
    if start == 0 {
        return Ok(());
    }
    debug_assert!(check_no_orphan_result(&messages, start));

    let Some(cut) = stored.get(start) else {
        return Ok(());
    };
    let previous = store.summary(session).await?;
    let summary = match summarize(store, llm, config, previous.as_deref(), &messages[..start]).await
    {
        Ok(text) => text,
        Err(error @ StoreError::BudgetExceeded { .. }) => return Err(error),
        Err(err) => {
            tracing::warn!(session = %session, error = %err, "compaction thất bại — giữ nguyên lịch sử");
            return Ok(());
        }
    };
    if summary.trim().is_empty() {
        return Ok(());
    }

    store.save_summary(session, summary.trim()).await?;
    store.delete_before(session, cut.seq).await?;
    tracing::info!(
        session = %session,
        removed = start,
        kept = messages.len().saturating_sub(start),
        "đã nén lịch sử hội thoại"
    );
    Ok(())
}
