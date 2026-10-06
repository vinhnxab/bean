//! MCP client stdio tích hợp Bean (agents.md mục 16).
//!
//! Mỗi server được spawn thành tiến trình con, handshake bằng `rmcp`, discovery qua
//! `tools/list`, rồi đăng ký từng tool thành `mcp__<server>__<tool>`. Server lỗi hoặc treo
//! chỉ bị log và bỏ qua; registry built-in vẫn dùng được để agent khởi động.

mod config;
mod connection;
mod runtime;
mod tool;

#[cfg(test)]
mod tests;

pub use config::{McpError, McpTimeouts};
pub use runtime::McpRuntime;
