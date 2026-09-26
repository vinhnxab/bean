//! Xây context gửi model (agents.md mục 8.2).
//!
//! Thứ tự đúng theo mục 8.2:
//! 1. system prompt + nội dung `MEMORY.md` và `USER.md` trong workspace (mỗi file tối đa
//!    [`MAX_MEMORY_FILE_CHARS`] ký tự);
//! 2. `sessions.summary` nếu có (do compaction ghi — mục 8.3);
//! 3. các message gần nhất **vừa ngân sách** `agent.context_budget_tokens`, cắt ở
//!    [`beanagent_memory::safe_cut`] nên không bao giờ tách cặp `assistant(tool_calls)`/`tool`.
//!
//! Đọc file bộ nhớ là **best-effort**: file chưa tồn tại (bình thường ở lần chạy đầu) hoặc
//! lỗi I/O chỉ khiến mục đó trống, không làm hỏng lượt chat.

use beanagent_memory::safe_cut::extend_start_backwards;
use beanagent_tools::WorkspaceFs;
use beanagent_types::config::Config;
use beanagent_types::{Message, SessionId};

use crate::store::{Store, StoreError};

/// Trần ký tự cho mỗi file bộ nhớ (mục 8.2). Dài hơn bị cắt tại **ranh giới ký tự**.
pub const MAX_MEMORY_FILE_CHARS: usize = 4_000;
/// Ước lượng token khi provider không có API đếm: `chars / 4` (mục 8.2).
const CHARS_PER_TOKEN: usize = 4;
/// Tên file bộ nhớ do agent/người dùng sửa (mục 8.1, 8.4).
pub const MEMORY_FILE: &str = "MEMORY.md";
/// File mô tả người dùng.
pub const USER_FILE: &str = "USER.md";

/// Context dựng xong cho một bước của vòng lặp agent.
#[derive(Debug, Clone)]
pub struct TurnContext {
    /// System prompt đầy đủ (đã gồm file bộ nhớ + summary khi có).
    pub system: String,
    /// Lịch sử trong ngân sách token, cũ → mới, đã cắt ở ranh giới an toàn.
    pub messages: Vec<Message>,
}

/// Dựng context cho `session` (mục 8.2).
///
/// `workspace = None` (registry chưa gắn workspace) ⇒ bỏ qua `MEMORY.md`/`USER.md`.
///
/// # Errors
/// [`StoreError`] khi đọc lịch sử/summary thất bại — lỗi đọc file bộ nhớ thì không.
pub async fn build(
    store: &dyn Store,
    config: &Config,
    session: SessionId,
    workspace: Option<&dyn WorkspaceFs>,
) -> Result<TurnContext, StoreError> {
    build_with_skills(store, config, session, workspace, "").await
}

/// Dựng context có kèm index skill cho progressive disclosure.
pub async fn build_with_skills(
    store: &dyn Store,
    config: &Config,
    session: SessionId,
    workspace: Option<&dyn WorkspaceFs>,
    skills_index: &str,
) -> Result<TurnContext, StoreError> {
    build_full(
        store,
        config,
        session,
        workspace,
        skills_index,
        config.agent.context_budget_tokens,
    )
    .await
}

/// Dựng context cho project profile cụ thể (M21.1).
///
/// `workspace` phải là workspace **của project đó** (xem
/// [`ToolRegistry::workspace_for`]) ⇒ `MEMORY.md`/`USER.md` của hai project không lẫn nhau.
/// `context_budget` cho phép áp ngân sách riêng theo role (M21.7).
pub async fn build_for_project(
    store: &dyn Store,
    config: &Config,
    session: SessionId,
    workspace: Option<&dyn WorkspaceFs>,
    skills_index: &str,
    context_budget: u32,
) -> Result<TurnContext, StoreError> {
    build_full(
        store,
        config,
        session,
        workspace,
        skills_index,
        context_budget,
    )
    .await
}

/// Thân chung của các hàm `build*`.
async fn build_full(
    store: &dyn Store,
    config: &Config,
    session: SessionId,
    workspace: Option<&dyn WorkspaceFs>,
    skills_index: &str,
    context_budget: u32,
) -> Result<TurnContext, StoreError> {
    let memory_md = workspace.map_or_else(String::new, |ws| read_memory_file(ws, MEMORY_FILE));
    let user_md = workspace.map_or_else(String::new, |ws| read_memory_file(ws, USER_FILE));
    let summary = store.summary(session).await?.unwrap_or_default();

    let mut system =
        crate::prompt::system_prompt(&config.agent, skills_index, &memory_md, &user_md);
    if !summary.trim().is_empty() {
        system.push_str("\n\n# Conversation summary\n");
        system.push_str(summary.trim());
    }

    let history = store.history(session, None, 0).await?;
    let messages = trim_history(&history, context_budget);
    Ok(TurnContext { system, messages })
}

/// Đọc một file bộ nhớ từ workspace, cắt ở trần ký tự; lỗi chỉ log và trả rỗng.
fn read_memory_file(workspace: &dyn WorkspaceFs, name: &str) -> String {
    match workspace.read_text(name) {
        Ok(text) => truncate_chars(&text, MAX_MEMORY_FILE_CHARS),
        Err(err) => {
            tracing::debug!(file = name, error = %err, "không đọc được file bộ nhớ (bỏ qua)");
            String::new()
        }
    }
}

/// Cắt chuỗi tại ranh giới ký tự (mục 22.9) — không panic với tiếng Việt/emoji.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Ước lượng độ dài (ký tự) mà một message chiếm trong context.
fn message_cost(message: &Message) -> usize {
    message.text_for_search().chars().count()
}

/// Chọn các message gần nhất vừa ngân sách token (mục 8.2 điểm 3).
///
/// * Message **mới nhất luôn được giữ**, kể cả khi một mình nó đã vượt ngân sách.
/// * Điểm cắt lùi thêm nếu cần để không tách cặp `assistant(tool_calls)`/`tool`
///   ([`extend_start_backwards`]) — có thể vượt ngân sách chút ít, an toàn hơn là hỏng
///   cặp tool và bị provider trả 400.
fn trim_history(history: &[Message], budget_tokens: u32) -> Vec<Message> {
    if history.is_empty() {
        return Vec::new();
    }
    let budget_chars = (budget_tokens as usize).saturating_mul(CHARS_PER_TOKEN);
    let mut used = 0usize;
    let mut keep = 0usize;
    for message in history.iter().rev() {
        let cost = message_cost(message);
        if keep > 0 && used + cost > budget_chars {
            break;
        }
        used += cost;
        keep += 1;
    }
    let start = extend_start_backwards(history, keep);
    history[start..].to_vec()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use beanagent_memory::safe_cut::check_no_orphan_result;
    use beanagent_security::CapWorkspace;
    use beanagent_tools::WorkspaceFs;
    use beanagent_types::{Config, Message, Role, SessionId, ToolCall};
    use tempfile::TempDir;

    use super::{MAX_MEMORY_FILE_CHARS, MEMORY_FILE, USER_FILE, build, trim_history};
    use crate::store::{MemoryStore, Store};

    fn workspace(dir: &TempDir) -> CapWorkspace {
        CapWorkspace::open(dir.path().to_path_buf()).unwrap()
    }

    /// Mục 8.2 điểm 1–2: system prompt chứa `MEMORY.md`, `USER.md` và summary của phiên.
    #[tokio::test]
    async fn system_prompt_includes_memory_files_and_summary() {
        let dir = TempDir::new().unwrap();
        let ws = workspace(&dir);
        ws.write_text(MEMORY_FILE, "Người dùng thích trà đá")
            .unwrap();
        ws.write_text(USER_FILE, "Tên: Vinh").unwrap();

        let store = MemoryStore::new();
        let session = SessionId::new(1);
        store.append(session, Message::user("chào")).await.unwrap();
        store.save_summary(session, "đang làm M5").await.unwrap();

        let ctx = build(&store, &Config::default(), session, Some(&ws))
            .await
            .unwrap();
        assert!(ctx.system.contains("Người dùng thích trà đá"));
        assert!(ctx.system.contains("Tên: Vinh"));
        assert!(ctx.system.contains("đang làm M5"));
        assert_eq!(ctx.messages.len(), 1);
    }

    /// File bộ nhớ thiếu (lần chạy đầu) không làm hỏng lượt chat.
    #[tokio::test]
    async fn missing_memory_files_are_ignored() {
        let dir = TempDir::new().unwrap();
        let ws = workspace(&dir);
        let store = MemoryStore::new();
        let ctx = build(&store, &Config::default(), SessionId::new(1), Some(&ws))
            .await
            .unwrap();
        assert!(ctx.system.contains("No persistent memory yet."));
        assert!(ctx.messages.is_empty());
        assert!(!ctx.system.contains("# Conversation summary"));
    }

    /// Cắt file bộ nhớ ở trần 4.000 ký tự **theo ký tự** (không panic với tiếng Việt).
    #[tokio::test]
    async fn memory_file_is_truncated_at_char_boundary() {
        let dir = TempDir::new().unwrap();
        let ws = workspace(&dir);
        ws.write_text(MEMORY_FILE, &"ế".repeat(MAX_MEMORY_FILE_CHARS + 500))
            .unwrap();

        let store = MemoryStore::new();
        let ctx = build(&store, &Config::default(), SessionId::new(1), Some(&ws))
            .await
            .unwrap();
        assert!(ctx.system.contains(&"ế".repeat(MAX_MEMORY_FILE_CHARS)));
        assert!(!ctx.system.contains(&"ế".repeat(MAX_MEMORY_FILE_CHARS + 1)));
    }

    /// Mục 8.2 điểm 3: chỉ gửi lịch sử vừa ngân sách token, và không tách cặp tool.
    #[tokio::test]
    async fn history_is_trimmed_to_budget_without_splitting_tool_pairs() {
        let store = MemoryStore::new();
        let session = SessionId::new(1);
        let filler = "x".repeat(200);
        for index in 0..6 {
            let call_id = format!("c{index}");
            store
                .append(session, Message::user(format!("{filler} hỏi {index}")))
                .await
                .unwrap();
            store
                .append(
                    session,
                    Message::assistant(
                        None,
                        vec![ToolCall::new(
                            call_id.clone(),
                            "probe",
                            serde_json::json!({ "n": index }),
                        )],
                    ),
                )
                .await
                .unwrap();
            store
                .append(
                    session,
                    Message::tool(call_id, format!("{filler} đáp {index}")),
                )
                .await
                .unwrap();
        }

        let mut config = Config::default();
        // 500 token = 2.000 ký tự ⇒ chỉ giữ được khoảng 4–5 lượt cuối (mỗi lượt ~432 ký tự).
        config.agent.context_budget_tokens = 500;
        let ctx = build(&store, &config, session, None).await.unwrap();
        assert!(
            ctx.messages.len() < 18 && ctx.messages.len() >= 12,
            "số message giữ lại không hợp lý: {}",
            ctx.messages.len()
        );
        assert!(check_no_orphan_result(&ctx.messages, 0));
        assert_ne!(ctx.messages[0].role, Role::Tool);
        assert_eq!(
            ctx.messages.last().unwrap().tool_call_id.as_deref(),
            Some("c5"),
            "phải luôn giữ message mới nhất"
        );
    }

    /// Ngân sách 0 (hoặc quá chật) vẫn giữ message mới nhất, và lùi để giữ đủ cặp tool.
    #[test]
    fn trim_keeps_newest_message_and_tool_pair() {
        let big = vec![Message::user("x".repeat(10_000))];
        assert_eq!(trim_history(&big, 1).len(), 1);

        let paired = vec![
            Message::user("a"),
            Message::assistant(
                None,
                vec![ToolCall::new("c1", "probe", serde_json::json!({}))],
            ),
            Message::tool("c1", "ok"),
        ];
        let kept = trim_history(&paired, 0);
        assert_eq!(kept.len(), 2, "phải lùi về assistant gọi ra tool");
        assert_eq!(kept[0].role, Role::Assistant);
        assert!(check_no_orphan_result(&kept, 0));
        assert!(trim_history(&[], 1_000).is_empty());
    }
}
