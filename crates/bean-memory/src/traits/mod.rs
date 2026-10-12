//! Trait Segregation modules following ISP
//!
//! Contains split traits from the large Store trait:
//! - SessionStore, MessageStore, MemoryStore, TaskStore
//! - UsageStore, OutboxStore, WebSessionStore, McpClientStore

// `traits::traits` là module trùng tên module cha (module_inception). Đây là cố ý để giữ
// đường dẫn `crate::traits::traits::…` ổn định; re-export `pub use traits::*` bên dưới cho
// phép import ngắn `crate::traits::…`. Cho phép lint cục bộ, phạm vi hẹp đúng một dòng.
#[allow(clippy::module_inception)]
pub mod traits;

pub use traits::*;
