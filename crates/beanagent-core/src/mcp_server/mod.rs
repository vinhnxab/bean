//! Bean làm **MCP server read-only** (milestone M25, `Plan.md` mục 5).
//!
//! # Mục tiêu
//!
//! Agent khác (Cline, Cursor, OpenCode, Claude Code…) gọi vào Bean như một MCP server
//! ngay trong phiên làm việc của chúng, thay vì phải chép sang Telegram/web.
//!
//! # Phạm vi cứng (không thương lượng — `Plan.md` M25)
//!
//! Chỉ expose tool mang **một trong ba tag** [`MCP_EXPOSED_TAGS`] và có
//! [`Risk::Safe`]. Mọi tool ghi/thực thi (`dev-write`, `infra-scan`, remediation
//! Nhóm 3, `marketing-publish`…) bị chặn **kể cả khi client tự xưng có quyền cao**
//! (role `admin` giữ tag `*` vẫn không vượt qua được cổng này — xem [`expose_gate`]).
//!
//! # Ba lớp phòng thủ
//!
//! 1. **Xác thực** ([`authenticate`]): token dài hạn riêng cho từng client, Bean chỉ lưu
//!    **hash** (SHA-256) trong `mcp_clients`. Tra ở bước `initialize` (handshake) — client
//!    không có token hợp lệ bị từ chối **trước** khi thấy bất kỳ tool nào.
//! 2. **Cổng expose** ([`expose_gate`]): allowlist tag + `Risk::Safe`, deny-by-default.
//!    Không dựa vào RBAC vì `admin` có tag `*`.
//! 3. **RBAC của role** ([`RolePermissions::allows`]): dùng lại **đúng hàm** M21 đã có,
//!    không viết logic lọc quyền riêng cho đường MCP (ràng buộc `Plan.md` mục 4.3).
//!
//! # Vì sao ở `beanagent-core`
//!
//! Cổng này cần [`Router`] (quyết định quyền), `ToolRegistry`, `AuditLog` và `Store` —
//! tất cả đã ở đây. Đặt cùng chỗ với `Router` giữ được ràng buộc "RBAC check nằm ở
//! đúng một điểm": [`Router::call_tool_as`] là điểm gọi thứ hai của *cùng* một hàm quyết
//! định [`Config::permissions_for`] + [`RolePermissions::allows`], không phải một bản
//! sao logic riêng.
//!
//! # Tham số từ client là input không tin cậy
//!
//! [`sanitize_client_args`] chạy **trước khi** tham số chạm tầng dưới: request đến từ
//! một coding agent "có vẻ đáng tin" vẫn là dữ liệu ngoài lõi, và `memory_query`/
//! `billing_read_cost` đều đưa tham số vào truy vấn nội bộ (xem `docs/known-issues.md`,
//! nguyên tắc "query của model là dữ liệu không tin cậy").

pub mod auth;
pub mod gate;
pub mod guard;
pub mod handler;
pub mod sanitize;
pub mod transport;

pub use auth::{McpAuth, McpClientIdentity, hash_token, new_token};
pub use gate::{ExposeDecision, MCP_EXPOSED_TAGS, expose_gate};
pub use guard::{LimitVerdict, McpRateLimiter};
pub use handler::{BeanMcpHandler, HandlerDeps};
pub use sanitize::sanitize_client_args;
pub use transport::{
    McpHttpService, McpServerError, ServeContext, bearer_token, host_allowlist, http_service,
    serve_stdio,
};
