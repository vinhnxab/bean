//! Nội dung **thẻ xác nhận** (mục 12.1.4): những gì người dùng đọc trước khi bấm
//! Duyệt/Từ chối.
//!
//! Tách khỏi `tool_call` vì đây là trách nhiệm dựng văn bản trình bày, không phải
//! điều phối thực thi — và module dưới `src/agent/` có trần 600 dòng
//! (tests/architecture.rs). Hai nguồn làm giàu thẻ:
//!
//! * **D27** — tool tự mô tả thay đổi qua [`bean_tools::Tool::confirm_details`]
//!   (diff của `write_file`/`edit_file`, cảnh báo `old` không khớp duy nhất),
//!   **thay thế** preview `k=v` khi đo được để content không hiện hai lần;
//! * **D26** — với `rollback_workspace`, danh sách thay đổi thật sẽ bị hoàn tác.

use super::*;

/// Dựng prompt xác nhận cho một tool call.
///
/// `tool` là tool đã tra từ registry (có thể `None` khi tool không tồn tại — khi đó
/// chỉ còn preview); `checkpoint` là handle clone từ `TurnState` tại thời điểm hỏi
/// (chỉ rollback dùng). Đo trên `workspace` **hiện tại** — file có thể đã khác từ lúc
/// model sinh args.
pub(super) async fn build_confirm_prompt(
    tool: Option<&dyn bean_tools::Tool>,
    workspace: &dyn bean_tools::WorkspaceFs,
    call: &ToolCall,
    args_preview: &str,
    checkpoint: Option<Arc<RunCheckpoint>>,
) -> String {
    let deny_note = deny_list_reason(&call.name, &call.args)
        .map(|reason| format!("\n[cảnh báo deny-list] {}", reason.label))
        .unwrap_or_default();
    let mut prompt = match tool.and_then(|tool| tool.confirm_details(workspace, &call.args)) {
        Some(details) => format!("{} {details}{deny_note}", call.name),
        None => format!("{} {}{deny_note}", call.name, args_preview),
    };
    // (D26) Thẻ xác nhận của rollback hiển thị **danh sách thay đổi thật** đo tại
    // thời điểm hỏi — model không thể che giấu những gì sẽ bị hoàn tác.
    if call.name == bean_tools::ROLLBACK_TOOL
        && let Some(handle) = checkpoint
    {
        match handle.changes(workspace.root()).await {
            Ok(changes) if changes.is_empty() => {
                prompt.push_str("\n\nKhông có thay đổi nào kể từ checkpoint.");
            }
            Ok(changes) => {
                prompt.push_str(&format!(
                    "\n\nSẽ hoàn tác về {}:\n{}",
                    handle.label(),
                    changes.summary()
                ));
            }
            Err(error) => {
                prompt.push_str(&format!("\n\nKhông so sánh được thay đổi: {error}"));
            }
        }
    }
    prompt
}
