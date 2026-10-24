//! Trait [`Store`] — hợp đồng lưu trữ mà agent loop chỉ cần biết (agents.md mục 8).
//!
//! # Vì sao đây là file riêng
//!
//! Đây là **bề mặt ổn định nhất của cả tầng bộ nhớ**: `agent::run_turn` và mọi adapter
//! chỉ làm việc với `&dyn Store`, nên thêm một bản cài đặt mới (Postgres, Redis, hay
//! một fake gọn cho test) không đòi sửa một dòng nào ở nơi gọi.
//!
//! Giữ nó tách riêng còn giúp câu hỏi "cài đặt mới cần làm gì" trả lời được bằng
//! cách đọc **một** file, thay vì phải đọc cả `memory.rs` lẫn `sqlite/` để biết
//! `MemoryStore` và `SqliteStore` khác nhau ở đâu.
//!
//! # Trait Segregation (ISP)
//!
//! Trait `Store` lớn (51 methods) đã được tách thành các traits nhỏ hơn trong module `solid`.
//! Để hoàn thành refactoring, cần thêm module solid/traits.rs với các trait được tách ra.
//!
//! # SOLID Principles
//!
//! - **I** - Interface Segregation: Clients không phải phụ thuộc vào methods họ không dùng
//! - **D** - Dependency Inversion: Dependencies qua traits, không concrete types

use bean_types::SessionId;

use super::types::StoreError;

// pub use crate::traits::traits::*;

#[async_trait::async_trait]
pub trait Store:
    crate::traits::traits::SessionStore
    + crate::traits::traits::MessageStore
    + crate::traits::traits::MemoryStore
    + crate::traits::traits::TaskStore
    + crate::traits::traits::UsageStore
    + crate::traits::traits::OutboxStore
    + crate::traits::traits::WebSessionStore
    + crate::traits::traits::McpClientStore
    + crate::traits::traits::ToolOverrideStore
    + Send
    + Sync
{
    /// Nén lịch sử khi vượt 70% `agent.context_budget_tokens` (agents.md mục 8.3).
    ///
    /// Tóm tắt các message cũ vào `sessions.summary` rồi xoá chúng, **giữ nguyên**
    /// mọi cặp `assistant(tool_calls)`/`tool` nhờ chọn điểm cắt ở ranh giới an toàn.
    async fn compact(
        &self,
        session: SessionId,
        llm: &dyn bean_llm::LlmProvider,
        config: &bean_types::Config,
    ) -> Result<(), StoreError>;
}
