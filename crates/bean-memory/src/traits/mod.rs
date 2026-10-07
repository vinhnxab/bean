//! Trait Segregation modules following ISP
//!
//! Contains split traits from the large Store trait:
//! - SessionStore, MessageStore, MemoryStore, TaskStore
//! - UsageStore, OutboxStore, WebSessionStore, McpClientStore

pub mod traits;
