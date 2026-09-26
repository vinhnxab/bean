//! # BeanAgent-tools
//!
//! Trait `Tool`, registry và các tool built-in + MCP (agents.md mục 7, 16).
//!
//! * **M3**: `trait Tool` (`spec`, `risk`, `call`), `ToolCtx`, `TypedTool<P>` (schema sinh bằng
//!   `schemars`, doc comment thành description, `deny_unknown_fields`), `ToolRegistry`, và nhóm
//!   tool file: `read_file`, `list_dir`, `glob`, `grep` (Safe), `write_file`, `edit_file` (Confirm).
//! * **M4**: path jail thật bằng cap-std — cài đặt duy nhất của [`WorkspaceFs`] nằm ở
//!   `beanagent_security::paths::CapWorkspace` (mọi tool chỉ gọi qua trait này).
//!   Tool `run_shell` + sandbox + policy/audit cũng nằm ở `beanagent-security`.
//! * **M7**: `web_fetch`, `web_search` (chống SSRF, bọc `<untrusted_content>`).
//! * **M14**: `mcp__<server>__<tool>` qua `rmcp`.
//!
//! Quy ước chung:
//! * `Risk` được định nghĩa ở `beanagent-types` (D5.9) và re-export ở đây để mọi crate dùng
//!   chung một kiểu.
//! * Schema của tool là JSON **thô** sinh từ `schemars`; provider chịu trách nhiệm chuẩn hoá
//!   trước khi gửi API (`beanagent-llm::schema::sanitize_tool_schema`, D6.10).
//! * Mọi thao tác I/O chặn (blocking) được bọc `tokio::task::spawn_blocking` (mục 22.8).
#![forbid(unsafe_code)]

pub mod builtin;
pub mod ctx;
pub mod error;
pub mod mcp;
pub mod registry;
pub mod text;
pub mod tool;
pub mod typed;
pub mod untrusted;
pub mod workspace;

pub use beanagent_types::{Risk, ToolSpec};
pub use ctx::{AlertSink, ToolCtx};
pub use error::ToolError;
pub use registry::ToolRegistry;
pub use text::{compile_regex, truncate_chars};
pub use tool::Tool;
pub use typed::{TypedTool, deserialize_params, typed_spec};
pub use untrusted::{
    CLOSE_TAG as UNTRUSTED_CLOSE_TAG, MAX_WRAPPED_OUTPUT_CHARS, OPEN_TAG as UNTRUSTED_OPEN_TAG,
    UntrustedFlag, contains_untrusted_block, escape_closing_tags, wrap as wrap_untrusted,
    wrap_bounded,
};
pub use workspace::{DirEntryInfo, GrepMatch, WorkspaceFs};
