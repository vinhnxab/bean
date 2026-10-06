//! Worker thread — nơi **duy nhất** được chạm vào connection SQLite (mục 22.8).
//!
//! # Vì sao tách riêng
//!
//! `worker_loop` + `dispatch` là **đối xứng chính xác** với `DbCommand`: mỗi biến thể
//! được nhận ở đây có đúng một nhánh xử lý. Đây cũng là **ranh giới không được vượt**:
//! mọi hàm trong `q_*.rs` đều là hàm đồng bộ nhận `&mut Connection`, và chỉ được gọi
//! từ `dispatch`.
//!
//! Nếu sau này thêm một lệnh mới, có **hai** chỗ phải sửa cùng lúc: thêm biến thể ở
//! `command.rs` và thêm nhánh ở đây. Compiler không bắt được sự thiếu sót thứ hai,
//! nên bất biến "mỗi lệnh có một nhánh" được giữ bằng kỷ luật review, không bằng kiểu.

use rusqlite::Connection;

use super::command::DbCommand;
use super::q_history_search::{delete_before, list_messages, load_history, memory_search};
use super::q_mcp::{create_mcp_client, delete_mcp_client, get_mcp_client, list_mcp_clients};
use super::q_memory_outbox::{
    delete_outbox, insert_outbound, load_due_outbox, memory_save_row, update_outbox_retry,
};
use super::q_message::{
    append_message, archive_session, clear_messages, count_messages, create_session_row,
    ensure_session, load_summary, save_summary,
};
use super::q_session::{
    delete_memory, delete_session, find_active_session, list_memories, list_message_records,
    list_sessions, load_session_info, load_session_summary, message_by_id, update_session,
};
use super::q_task::{
    claim_task, create_task, delete_task, delete_task_for_session, due_tasks, list_tasks,
    list_tasks_for_session, set_task_enabled, set_task_status,
};
use super::q_usage_web::{
    add_usage, add_usage_by_role, create_web_session, delete_all_web_sessions, delete_web_session,
    get_web_session, read_usage, read_usage_by_role, touch_web_session,
};

// ---------------------------------------------------------------------------
// Worker thread — nơi duy nhất chạm vào SQLite
// ---------------------------------------------------------------------------

pub(super) fn worker_loop(mut conn: Connection, rx: std::sync::mpsc::Receiver<DbCommand>) {
    while let Ok(cmd) = rx.recv() {
        dispatch(&mut conn, cmd);
    }
    tracing::debug!("worker bộ nhớ đã dừng");
}

/// Thực thi một lệnh và gửi kết quả về; lỗi SQLite **không** làm chết worker.
pub(super) fn dispatch(conn: &mut Connection, cmd: DbCommand) {
    match cmd {
        DbCommand::EnsureSession {
            channel,
            chat_id,
            user_id,
            title,
            reply,
        } => {
            let _ = reply.send(ensure_session(conn, &channel, &chat_id, &user_id, &title));
        }
        DbCommand::CreateSession {
            channel,
            chat_id,
            user_id,
            title,
            reply,
        } => {
            let _ = reply.send(create_session_row(
                conn, &channel, &chat_id, &user_id, &title,
            ));
        }
        DbCommand::ArchiveSession { session, reply } => {
            let _ = reply.send(archive_session(conn, session));
        }
        DbCommand::Append {
            session,
            message,
            reply,
        } => {
            let _ = reply.send(append_message(conn, session, &message));
        }
        DbCommand::History {
            session,
            before_seq,
            limit,
            reply,
        } => {
            let _ = reply.send(load_history(conn, session, before_seq, limit));
        }
        DbCommand::Count { session, reply } => {
            let _ = reply.send(count_messages(conn, session));
        }
        DbCommand::Clear { session, reply } => {
            let _ = reply.send(clear_messages(conn, session));
        }
        DbCommand::SessionInfo { session, reply } => {
            let _ = reply.send(load_session_info(conn, session));
        }
        DbCommand::SaveSummary {
            session,
            summary,
            reply,
        } => {
            let _ = reply.send(save_summary(conn, session, &summary));
        }
        DbCommand::Summary { session, reply } => {
            let _ = reply.send(load_summary(conn, session));
        }
        DbCommand::SessionSummary { session, reply } => {
            let _ = reply.send(load_session_summary(conn, session));
        }
        DbCommand::FindActiveSession {
            channel,
            chat_id,
            reply,
        } => {
            let _ = reply.send(find_active_session(conn, &channel, &chat_id));
        }
        DbCommand::UpdateSession {
            session,
            user_id,
            title,
            archived,
            reply,
        } => {
            let _ = reply.send(update_session(
                conn,
                session,
                &user_id,
                title.as_deref(),
                archived,
            ));
        }
        DbCommand::DeleteSession {
            session,
            user_id,
            reply,
        } => {
            let _ = reply.send(delete_session(conn, session, &user_id));
        }
        DbCommand::ListSessions {
            user_id,
            query,
            archived,
            limit,
            reply,
        } => {
            let _ = reply.send(list_sessions(
                conn,
                &user_id,
                query.as_deref(),
                archived,
                limit,
            ));
        }
        DbCommand::MessageById { id, reply } => {
            let _ = reply.send(message_by_id(conn, id));
        }
        DbCommand::ListMessageRecords {
            session,
            before_seq,
            limit,
            reply,
        } => {
            let _ = reply.send(list_message_records(conn, session, before_seq, limit));
        }
        DbCommand::ListMemories {
            query,
            limit,
            reply,
        } => {
            let _ = reply.send(list_memories(conn, query.as_deref(), limit));
        }
        DbCommand::DeleteMemory { id, reply } => {
            let _ = reply.send(delete_memory(conn, id));
        }
        DbCommand::ListTasks { reply } => {
            let _ = reply.send(list_tasks(conn));
        }
        DbCommand::ListTasksForSession { session, reply } => {
            let _ = reply.send(list_tasks_for_session(conn, session));
        }
        DbCommand::CreateTask { task, reply } => {
            let _ = reply.send(create_task(conn, task));
        }
        DbCommand::SetTaskEnabled { id, enabled, reply } => {
            let _ = reply.send(set_task_enabled(conn, id, enabled));
        }
        DbCommand::DeleteTaskForSession { id, session, reply } => {
            let _ = reply.send(delete_task_for_session(conn, id, session));
        }
        DbCommand::DeleteTask { id, reply } => {
            let _ = reply.send(delete_task(conn, id));
        }
        DbCommand::DueTasks { now, limit, reply } => {
            let _ = reply.send(due_tasks(conn, &now, limit));
        }
        DbCommand::ClaimTask {
            id,
            expected_next_run,
            next_run,
            last_run_at,
            status,
            reply,
        } => {
            let _ = reply.send(claim_task(
                conn,
                id,
                &expected_next_run,
                &next_run,
                &last_run_at,
                &status,
            ));
        }
        DbCommand::SetTaskStatus { id, status, reply } => {
            let _ = reply.send(set_task_status(conn, id, &status));
        }
        DbCommand::AddUsage { day, usage, reply } => {
            let _ = reply.send(add_usage(conn, &day, usage));
        }
        DbCommand::Usage { day, reply } => {
            let _ = reply.send(read_usage(conn, &day));
        }
        DbCommand::AddUsageByRole {
            day,
            role,
            usage,
            reply,
        } => {
            let _ = reply.send(add_usage_by_role(conn, &day, &role, usage));
        }
        DbCommand::UsageByRole { day, role, reply } => {
            let _ = reply.send(read_usage_by_role(conn, &day, &role));
        }
        DbCommand::CreateWebSession {
            token_hash,
            user_id,
            created_at,
            expires_at,
            reply,
        } => {
            let _ = reply.send(create_web_session(
                conn,
                &token_hash,
                &user_id,
                &created_at,
                &expires_at,
            ));
        }
        DbCommand::GetWebSession {
            token_hash,
            now,
            reply,
        } => {
            let _ = reply.send(get_web_session(conn, &token_hash, &now));
        }
        DbCommand::TouchWebSession {
            token_hash,
            now,
            reply,
        } => {
            let _ = reply.send(touch_web_session(conn, &token_hash, &now));
        }
        DbCommand::DeleteWebSession { token_hash, reply } => {
            let _ = reply.send(delete_web_session(conn, &token_hash));
        }
        DbCommand::DeleteAllWebSessions { reply } => {
            let _ = reply.send(delete_all_web_sessions(conn));
        }
        DbCommand::CreateMcpClient {
            token_hash,
            name,
            role,
            created_at,
            expires_at,
            reply,
        } => {
            let _ = reply.send(create_mcp_client(
                conn,
                &token_hash,
                &name,
                &role,
                &created_at,
                &expires_at,
            ));
        }
        DbCommand::GetMcpClient {
            token_hash,
            now,
            reply,
        } => {
            let _ = reply.send(get_mcp_client(conn, &token_hash, &now));
        }
        DbCommand::ListMcpClients { reply } => {
            let _ = reply.send(list_mcp_clients(conn));
        }
        DbCommand::DeleteMcpClient { name, reply } => {
            let _ = reply.send(delete_mcp_client(conn, &name));
        }
        DbCommand::MemorySave { text, tags, reply } => {
            let _ = reply.send(memory_save_row(conn, &text, &tags));
        }
        DbCommand::MemorySearch { query, reply } => {
            let _ = reply.send(memory_search(conn, &query));
        }
        DbCommand::Outbound {
            channel,
            chat_id,
            payload,
            next_attempt_at,
            reply,
        } => {
            let _ = reply.send(insert_outbound(
                conn,
                &channel,
                &chat_id,
                &payload,
                &next_attempt_at,
            ));
        }
        DbCommand::DueOutbox { now, limit, reply } => {
            let _ = reply.send(load_due_outbox(conn, &now, limit));
        }
        DbCommand::CompleteOutbox { id, reply } => {
            let _ = reply.send(delete_outbox(conn, id));
        }
        DbCommand::RetryOutbox {
            id,
            next_attempt_at,
            last_error,
            reply,
        } => {
            let _ = reply.send(update_outbox_retry(conn, id, &next_attempt_at, &last_error));
        }
        DbCommand::ListMessages {
            session,
            before_seq,
            limit,
            reply,
        } => {
            let _ = reply.send(list_messages(conn, session, before_seq, limit));
        }
        DbCommand::DeleteBefore {
            session,
            before_seq,
            reply,
        } => {
            let _ = reply.send(delete_before(conn, session, before_seq));
        }
    }
}
