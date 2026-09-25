//! # BeanAgent-types
//!
//! Kiểu dữ liệu **trung lập với provider** và cấu hình dùng chung cho toàn bộ BeanAgent
//! (agents.md mục 5, 18). Mọi provider tự chuyển đổi `Message` ⇄ định dạng riêng của API.
#![forbid(unsafe_code)]

pub mod config;
pub mod ids;
pub mod llm;
pub mod message;
pub mod router;
pub mod tool;

pub use config::{Config, ConfigError, ResolvedSecrets};
pub use ids::{ConfirmId, RunId, SessionId};
pub use llm::{LlmDelta, LlmResponse, LlmToolCallDelta, StopReason, Usage};
pub use message::{Message, Role, ToolCall};
pub use router::{ConfirmOutcome, Outbound, OutboundAction, OutboundKind, RunEvent};
pub use tool::{Risk, ToolSpec};
