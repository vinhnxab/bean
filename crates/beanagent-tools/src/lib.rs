//! # BeanAgent-tools
//!
//! Trait `Tool`, registry và các tool built-in + MCP (agents.md mục 7, 16).
//!
//! * **M3**: `trait Tool` (`spec`, `risk`, `call`), `ToolCtx`, `TypedTool<P>` (schema sinh bằng
//!   `schemars`, doc comment thành description, `deny_unknown_fields`), `ToolRegistry`, và nhóm
//!   tool file: `read_file`, `list_dir`, `glob`, `grep` (Safe), `write_file`, `edit_file` (Confirm).
//! * **M4**: `run_shell` (Confirm, chạy trong sandbox), policy + audit.
//! * **M7**: `web_fetch`, `web_search` (chống SSRF, bọc `<untrusted_content>`).
//! * **M14**: `mcp__<server>__<tool>` qua `rmcp`.
#![forbid(unsafe_code)]
