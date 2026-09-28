//! # bean-core
//!
//! Lõi agent: vòng lặp, context, Router, `trait Channel`, scheduler, learning loop
//! (agents.md mục 6, 10, 14, 17).
//!
//! * **M3**: `agent::run_turn` (`max_steps`, lỗi tool ⇒ `is_error`, timeout từng tool, cắt output
//!   tại ranh giới UTF-8, chống lặp, `CancellationToken`), `trait RunIo` (do Router sở hữu —
//!   `docs/decisions.md` D1.2), `trait Store` bản in-memory, `agent::prompt` theo mục 19.
//! * **M5**: `context::build` dựng context theo mục 8.2 (system prompt + `MEMORY.md`/`USER.md`
//!   + `sessions.summary` + lịch sử vừa ngân sách token, cắt ở ranh giới an toàn).
//! * **M8**: `Router` (hàng đợi theo session, `submit`/`events`/`resolve_confirm`/`cancel`/`notify`,
//!   `RunEvent`, `Outbound`) + slash command xử lý trong lõi, `allowed_users`.
//! * **M13**: `scheduler` (tick 30 giây, `trait Clock`, croner + chrono-tz, outbox retry).
//! * **M15**: learning loop (reflection sinh skill nháp, chỉ kích hoạt khi người dùng duyệt).
//! * **M25**: [`mcp_server`] — Bean đóng vai **MCP server read-only** cho agent khác
//!   (Cline/Cursor/OpenCode/Claude Code). Cổng expose cứng + tái dùng `RolePermissions`.
//!
//! Trách nhiệm của crate này là **quyền quyết định**: run thuộc Router chứ không thuộc kết nối,
//! confirm do Router cấp `confirm_id` và phân giải, huỷ chỉ khi có yêu cầu tường minh.
#![forbid(unsafe_code)]

pub mod agent;
pub mod context;
mod learning;
pub mod mcp_server;
pub mod prompt;
pub mod router;
pub mod run_io;
pub mod scheduler;
pub mod store;

pub use agent::{EndReason, RunOutcome, RunTurnArgs, run_turn, run_turn_outcome};
pub use context::TurnContext;
pub use prompt::system_prompt;
pub use router::{
    Channel, Incoming, PendingConfirmInfo, Router, RouterDeps, RouterError, RouterOptions,
    RouterSnapshot, RunningInfo,
};
pub use run_io::{Decision, RunIo};
pub use scheduler::{
    Clock, FixedClock, Scheduler, SchedulerError, SystemClock, next_run_after, schedule_tools,
};
pub use store::{MemoryStore, SqliteStore, Store, memory_tools};
