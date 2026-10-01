//! Kiểu dữ liệu public của REST/WebSocket.
//!
//! Đây là nguồn duy nhất cho binding TypeScript; không định nghĩa song song trong web UI.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Tên cookie phiên đăng nhập.
pub const SESSION_COOKIE: &str = "bean_session";

/// Request đăng nhập.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct LoginRequest {
    /// Mật khẩu người dùng.
    pub password: String,
}

/// Response đăng nhập tối thiểu, không chứa token.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LoginResponse {
    /// User ID hiện tại.
    pub user_id: String,
}

/// Response kiểm tra phiên.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuthMeResponse {
    /// User ID hiện tại.
    pub user_id: String,
}

/// Response logout.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogoutResponse {
    /// Luôn true sau khi xoá session.
    pub ok: bool,
}

/// Response trạng thái runtime.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatusResponse {
    /// Phiên bản binary.
    pub version: String,
    /// Model đang dùng.
    pub model: String,
    /// Trần số bước của một lượt chat.
    pub max_steps: u32,
    /// Ngân sách token/ngày.
    pub daily_token_budget: u64,
    /// Token đã dùng hôm nay.
    pub tokens_used: u64,
    /// Uptime tính bằng giây.
    pub uptime_seconds: u64,
    /// Các channel đang chạy.
    pub channels: Vec<String>,
}

/// Lỗi API ổn định.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiError {
    /// Mã ổn định cho client.
    pub code: String,
    /// Thông bố an toàn cho client.
    pub message: String,
}

/// Metadata session trả về REST.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionDto {
    /// ID session.
    pub id: i64,
    /// Channel sở hữu session.
    pub channel: String,
    /// Chat ID.
    pub chat_id: String,
    /// User ID.
    pub user_id: String,
    /// Tiêu đề.
    pub title: String,
    /// Đã archive hay chưa.
    pub archived: bool,
    /// Thời điểm tạo.
    pub created_at: String,
    /// Thời điểm cập nhật.
    pub updated_at: String,
}

/// Danh sách session.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionListResponse {
    /// Các session.
    pub sessions: Vec<SessionDto>,
}

/// Request tạo session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct CreateSessionRequest {
    /// Tiêu đề tùy chọn.
    pub title: Option<String>,
}

/// Request cập nhật session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct UpdateSessionRequest {
    /// Tiêu đề mới nếu có.
    pub title: Option<String>,
    /// Trạng thái archive nếu có.
    pub archived: Option<bool>,
}

/// Message REST; `message` là JSON canonical của Message.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageDto {
    /// ID message.
    pub id: i64,
    /// Session chứa message.
    pub session_id: i64,
    /// Thứ tự trong session.
    pub seq: u64,
    /// Nội dung canonical JSON.
    pub message: Value,
    /// Thời điểm ghi.
    pub created_at: String,
}

/// Trang message.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MessageListResponse {
    /// Các message.
    pub messages: Vec<MessageDto>,
}

/// Query session.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export)]
pub struct SessionQuery {
    /// Tìm trong title.
    pub q: Option<String>,
    /// Lọc archive.
    pub archived: Option<bool>,
    /// Giới hạn số kết quả.
    pub limit: Option<usize>,
}

/// Query message.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export)]
pub struct MessageQuery {
    /// Chỉ lấy seq trước giá trị này.
    pub before: Option<u64>,
    /// Giới hạn số kết quả.
    pub limit: Option<usize>,
}
/// Query danh sách ghi nhớ.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export)]
pub struct MemoryQuery {
    /// Từ khoá tìm kiếm.
    pub q: Option<String>,
    /// Giới hạn kết quả.
    pub limit: Option<usize>,
}

/// Nội dung file bộ nhớ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryFileResponse {
    /// Tên logic MEMORY hoặc USER.
    pub name: String,
    /// Nội dung.
    pub content: String,
}

/// Request cập nhật file bộ nhớ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct MemoryFileRequest {
    /// Nội dung mới.
    pub content: String,
}

/// Ghi nhớ dài hạn.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryDto {
    /// ID ghi nhớ.
    pub id: u64,
    /// Nội dung.
    pub text: String,
    /// Tag.
    pub tags: String,
    /// Thời điểm tạo.
    pub created_at: String,
}

/// Danh sách ghi nhớ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryListResponse {
    /// Các ghi nhớ.
    pub memories: Vec<MemoryDto>,
}

/// Skill tóm tắt.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillSummary {
    /// Tên skill.
    pub name: String,
    /// Mô tả.
    pub description: String,
}

/// Skill đầy đủ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillDetail {
    /// Tên skill.
    pub name: String,
    /// Mô tả.
    pub description: String,
    /// Nội dung SKILL.md.
    pub content: String,
    /// Đường dẫn thư mục.
    pub directory: String,
}

/// Danh sách skill.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillListResponse {
    /// Các skill.
    pub skills: Vec<SkillSummary>,
}

/// Một dòng audit đã redact.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditEntryDto {
    /// Timestamp.
    pub ts: String,
    /// Session ID.
    pub session: i64,
    /// Channel.
    pub channel: String,
    /// Tool hoặc loại sự kiện.
    pub tool: String,
    /// Tham số đã redact.
    pub args: Value,
    /// Kết quả nếu có.
    pub ok: Option<bool>,
    /// Quyết định.
    pub decision: String,
    /// Người quyết định.
    pub decided_by: String,
    /// Lỗi đã lọc.
    pub error: Option<String>,
}

/// Query audit.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[ts(export)]
pub struct AuditQuery {
    /// Timestamp/offset nội bộ cho phân trang.
    pub before: Option<u64>,
    /// Giới hạn số dòng.
    pub limit: Option<usize>,
}

/// Trang audit.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AuditListResponse {
    /// Các dòng audit.
    pub entries: Vec<AuditEntryDto>,
}

/// Response 501 có kiểu rõ ràng cho phần milestone sau.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NotImplementedResponse {
    /// Mã lỗi.
    pub code: String,
    /// Thông bố.
    pub message: String,
}

/// Response chung cho thao tác xoá.
/// Request tạo/cập nhật tác vụ định kỳ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct TaskRequest {
    /// Session sẽ chạy task; nếu bỏ trống, server dùng session đang hoạt động của channel/chat.
    #[serde(default)]
    pub session_id: Option<i64>,
    /// Cron theo timezone cấu hình của agent, ví dụ `0 7 * * *`.
    pub cron: String,
    /// Prompt sẽ chạy.
    pub prompt: String,
    /// Channel nhận kết quả.
    pub channel: String,
    /// Chat ID nhận kết quả.
    pub chat_id: String,
    /// Danh sách tool được phép chạy tự động.
    pub allowed_tools: Vec<String>,
    /// Tác vụ có bật hay không.
    pub enabled: Option<bool>,
}

/// Tác vụ định kỳ trả về REST.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskDto {
    /// ID tác vụ.
    pub id: u64,
    /// Session sở hữu tác vụ.
    pub session_id: i64,
    /// Cron.
    pub cron: String,
    /// Prompt.
    pub prompt: String,
    /// Channel.
    pub channel: String,
    /// Chat ID.
    pub chat_id: String,
    /// Tool được phép.
    pub allowed_tools: Vec<String>,
    /// Lần chạy kế tiếp UTC.
    pub next_run: String,
    /// Đang bật hay không.
    pub enabled: bool,
    /// Thời điểm tạo UTC.
    pub created_at: String,
    /// Lần chạy gần nhất UTC nếu có.
    pub last_run_at: Option<String>,
    /// Trạng thái lần chạy gần nhất.
    pub last_status: String,
}

/// Danh sách tác vụ định kỳ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskListResponse {
    /// Các tác vụ.
    pub tasks: Vec<TaskDto>,
}

/// Request bật/tắt một tác vụ định kỳ.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct TaskUpdateRequest {
    /// Tác vụ có được bật hay không.
    pub enabled: bool,
}

/// Một skill nháp do learning loop đề xuất.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillDraftDto {
    /// ID nháp ngẫu nhiên.
    pub id: String,
    /// Tên skill.
    pub name: String,
    /// `new` hoặc `update`.
    pub kind: String,
    /// Description của skill được đề xuất.
    pub description: String,
    /// Nội dung đầy đủ với skill mới, unified diff với skill sửa.
    pub content: String,
    /// Lý do reflection đưa ra.
    pub reason: String,
    /// Trạng thái duyệt.
    pub status: String,
    /// Timestamp tạo RFC3339 UTC.
    pub created_at: String,
}

/// Danh sách skill nháp.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillDraftListResponse {
    /// Các nháp.
    pub drafts: Vec<SkillDraftDto>,
}

/// Empty JSON body for draft approve/reject; the draft id is in the path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SkillDraftDecisionRequest {}

/// Kết quả duyệt/từ chối skill nháp.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SkillDraftDecisionResponse {
    /// ID nháp.
    pub id: String,
    /// Trạng thái sau quyết định.
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeleteResponse {
    /// Đã xoá hay không.
    pub deleted: bool,
}

/// Response thành công tối giểu.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OkResponse {
    /// Thao tác thành công.
    pub ok: bool,
}

/// Mức rủi ro wire.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RiskDto {
    Safe,
    Confirm,
    Dangerous,
}

/// Quyết định xác nhận.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionDto {
    Allow,
    AllowInSession,
    Deny,
}

/// Trạng thái sống của agent, ánh xạ từ `bean_types::AgentStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentStatusDto {
    /// Rảnh.
    Idle,
    /// Đang chạy.
    Working,
    /// Đang chờ người dùng duyệt.
    AwaitingYou,
}

/// Quan hệ kiến trúc với Manager, ánh xạ từ `bean_types::AgentRelation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentRelationDto {
    /// Manager điều phối.
    Manages,
    /// Review kiểu four-eyes, không tự duyệt.
    Reviews,
    /// Cảnh báo thẳng tới người quản trị, không qua Manager.
    AlertsDirectly,
}

/// Báo cáo chuẩn hoá `{status, summary, risks}` của một agent.
///
/// Đây là **báo cáo đã chuẩn hoá**, không phải dữ liệu thô: UI không có
/// đường nào để đọc log hay output tool của agent con.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentReportDto {
    /// Tên role.
    pub role: String,
    /// Trạng thái sống.
    pub status: AgentStatusDto,
    /// Một dòng tóm tắt.
    pub summary: String,
    /// Rủi ro đang mở.
    pub risks: Vec<String>,
    /// Quan hệ với Manager để UI vẽ đúng hình học.
    pub relation: AgentRelationDto,
}

/// Danh sách agent đã được lọc theo RBAC của người gọi.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentListResponse {
    /// Các agent mà vai trò của người gọi được phép thấy.
    pub agents: Vec<AgentReportDto>,
    /// Vai trò đã resolve của người gọi, để UI giải thích vì sao chỉ thấy phần này.
    pub viewer_role: String,
}

/// Run đang chạy trong snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RunningInfo {
    pub session_id: i64,
    pub run_id: String,
    /// Role sở hữu run; `null` khi RBAC tắt.
    pub role: Option<String>,
}

/// Confirm đang chờ trong snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PendingConfirm {
    pub confirm_id: String,
    pub session_id: i64,
    pub run_id: String,
    pub prompt: String,
    pub risk: RiskDto,
    pub allow_session_option: bool,
    pub timeout_seconds: u32,
    /// Role của agent đang chờ duyệt; `null` khi RBAC tắt.
    pub role: Option<String>,
}

/// Message client gửi lên WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ClientMsg {
    Start {
        session_id: i64,
        text: String,
    },
    Cancel {
        session_id: i64,
    },
    Confirm {
        confirm_id: String,
        decision: DecisionDto,
    },
    Ping,
}

/// Message server gửi qua WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ServerMsg {
    Sync {
        running: Vec<RunningInfo>,
        pending_confirms: Vec<PendingConfirm>,
    },
    Queued {
        session_id: i64,
        run_id: String,
        position: u32,
    },
    Text {
        session_id: i64,
        run_id: String,
        text: String,
    },
    TextDelta {
        session_id: i64,
        run_id: String,
        text: String,
        index: u32,
        reset: bool,
    },
    ToolStart {
        session_id: i64,
        run_id: String,
        id: String,
        tool: String,
        summary: String,
        args_preview: String,
    },
    ToolEnd {
        session_id: i64,
        run_id: String,
        id: String,
        ok: bool,
        output_preview: String,
    },
    ConfirmRequest {
        session_id: i64,
        run_id: String,
        confirm_id: String,
        prompt: String,
        risk: RiskDto,
        allow_session_option: bool,
        timeout_seconds: u32,
        /// Role của agent đang chờ duyệt; `null` khi RBAC tắt.
        role: Option<String>,
    },
    ConfirmResolved {
        confirm_id: String,
        outcome: String,
    },
    Final {
        session_id: i64,
        run_id: String,
        message_id: i64,
    },
    Error {
        session_id: Option<i64>,
        run_id: Option<String>,
        code: String,
        message: String,
    },
    Notification {
        session_id: i64,
        message_id: i64,
    },
    Pong,
}

// ───────────────────────────── Tools (read-only) ────────────────────────────

/// Một tool đã đăng ký trong registry, phục vụ màn **Tools**.
///
/// # Vì sao `risk` là mức *khởi điểm*
///
/// `Tool::risk` nhận `args` — một số tool đổi mức rủi ro **theo tham số**
/// (`run_shell` với lệnh nguy hiểm là `Dangerous`, với `ls` là `Confirm`).
/// Ở đây không có lời gọi thật nào để xét, nên dùng tham số rỗng — cùng cách
/// `bean_core::mcp_server::gate::expose_gate` đang dùng, để hai nơi không lệch nhau.
/// UI vì thế phải hiển thị nhãn "mức rủi ro mặc định", không khẳng định mọi lời
/// gọi đều ở mức đó.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolDto {
    /// Tên tool đúng như model nhìn thấy (`read_file`, `mcp__<server>__<tool>`).
    pub name: String,
    /// Mô tả tool — chính là text gửi cho model, nên màn Tools là nơi duy nhất
    /// người dùng đọc được "Bean tự hiểu tool này để làm gì".
    pub description: String,
    /// Mức rủi ro với tham số rỗng (xem docs ở [`ToolDto`]).
    pub risk: RiskDto,
    /// `"builtin"` hoặc `"mcp"`.
    pub source: String,
    /// Tên MCP server khi `source = "mcp"`; `None` với tool built-in.
    pub mcp_server: Option<String>,
    /// Tag RBAC mà role phải giữ **ít nhất một** để thấy/gọi tool. Rỗng ⇒ tool
    /// untagged: mọi role đã cấp quyền đều thấy (role `no-access` vẫn không thấy).
    pub required_tags: Vec<String>,
    /// Tag bổ sung (`also_visible_to`): mở một tool untagged cho role cụ thể (M24).
    pub extra_tags: Vec<String>,
    /// Tool trả nội dung từ nguồn ngoài lõi và được bọc `<untrusted_content>`
    /// (mục 15.4) — lý do nó có thể dẫn tới prompt injection.
    pub untrusted: bool,
}

/// Response của `GET /api/tools`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolListResponse {
    /// Tool mà **người gọi** được thấy, đã sort theo tên.
    ///
    /// Lọc xảy ra ở server trước khi serialise: tool bị chặn không xuất hiện
    /// trong payload (RBAC không phải việc của UI — xem `list_agents`).
    pub tools: Vec<ToolDto>,
    /// Tổng số tool đã đăng ký, kể cả tool người gọi không được thấy.
    pub total: usize,
    /// Role của người gọi — UI dùng để giải thích vì sao danh sách bị cắt.
    pub viewer_role: String,
    /// RBAC có bật không (`agent.user_roles` khác rỗng). `false` ⇒ một người dùng,
    /// mọi tool đều thấy.
    pub rbac_enabled: bool,
}

// ───────────────────────────── MCP servers (read-only) ──────────────────────

/// Một `[[mcp_servers]]` kèm trạng thái kết nối **suy ra từ registry**.
///
/// # Trạng thái lấy từ đâu
///
/// `McpRuntime` giữ connection nhưng không expose trạng thái, và thêm API đó sẽ
/// chạm vào đường khởi động. Trong khi đó `McpRuntime::register_server` chỉ đăng
/// ký tool **sau khi** initialize + discovery thành công, nên "có bao nhiêu tool
/// `mcp__<server>__*` trong registry" là bằng chứng trung thực nhất: `> 0` nghĩa
/// là server đã nói chuyện được với Bean.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpServerDto {
    /// Tên server (tiền tố `mcp__<name>__`).
    pub name: String,
    /// Lệnh khởi chạy — có thể là `docker run …` để bọc server không tin cậy.
    pub command: String,
    /// Tham số dòng lệnh.
    pub args: Vec<String>,
    /// `true` ⇒ tool của server hạ xuống `Safe`; `false` ⇒ mọi tool là `Confirm`.
    pub trusted: bool,
    /// Trần thời gian cho **một lần gọi tool** của server này (giây).
    pub call_timeout_seconds: Option<u32>,
    /// Tag RBAC áp cho mọi tool của server (M22).
    pub required_tags: Vec<String>,
    /// Số tool của server đang có trong registry (0 ⇒ chưa kết nối hoặc không có tool).
    pub tool_count: usize,
    /// `tool_count > 0`: server đã kết nối và discovery thành công.
    pub connected: bool,
}

/// Response của `GET /api/mcp`.
///
/// **Không** chứa `env`: giá trị biến môi trường của MCP server là secret (mục 15.6),
/// và ngay tên biến cũng gợi ý loại credential đang dùng — UI không cần chúng.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpServerListResponse {
    /// Danh sách server theo thứ tự khai báo trong `bean.toml`.
    pub servers: Vec<McpServerDto>,
}

// ───────────────────────────── Usage (read-only) ────────────────────────────

/// Token đã dùng trong **một ngày** (chốt UTC — cùng khoá với bảng `usage_by_role`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageDayDto {
    /// Ngày dạng `YYYY-MM-DD`.
    pub day: String,
    /// Token đầu vào.
    pub input_tokens: u64,
    /// Token đầu ra.
    pub output_tokens: u64,
    /// Tổng hai chiều — so với `daily_token_budget` để tính mức đã dùng.
    pub total_tokens: u64,
}

/// Query của `GET /api/usage`.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct UsageQuery {
    /// Số ngày gần nhất (tính cả hôm nay). Mặc định 14, tối đa 31 — biểu đồ
    /// theo ngày không cần dài hơn, và mỗi ngày là một query SQLite.
    #[serde(default)]
    pub days: Option<i64>,
}

/// Response của `GET /api/usage`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UsageListResponse {
    /// Các ngày gần nhất theo thứ tự **tăng dần** (ngày cũ nhất trước).
    pub days: Vec<UsageDayDto>,
    /// Ngân sách token/ngày (`security.daily_token_budget`).
    pub daily_token_budget: u64,
    /// Token đã dùng hôm nay — bằng phần tử cuối của `days` khi hôm nay có dữ liệu.
    pub today_tokens: u64,
}
// ───────────────────────────── System / config (read-only) ──────────────────

/// Thông tin sandbox (`[security.sandbox]`) cho màn Status.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SandboxDto {
    /// `"docker"` hoặc `"host"` — `host` nghĩa là **mọi** lệnh shell là `Dangerous`.
    pub mode: String,
    /// Image dùng cho container sandbox.
    pub image: String,
    /// Container có được ra mạng không (mặc định: không).
    pub network: bool,
    /// Trần bộ nhớ (`docker --memory`).
    pub memory: String,
    /// Số CPU được dùng.
    pub cpus: f32,
    /// Trần số tiến trình (chống fork bomb).
    pub pids_limit: u32,
    /// Thời gian tối đa cho một lệnh shell (giây).
    pub timeout_seconds: u64,
}

/// Một role trong `[[roles]]` — màn Status giải thích RBAC bằng dữ liệu thật.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RoleDto {
    /// Tên role.
    pub name: String,
    /// Tag được cấp (`["*"]` = toàn quyền).
    pub tool_tags: Vec<String>,
    /// Tag bị **cấm cứng** (four-eyes, M21.6).
    pub forbid_tags: Vec<String>,
    /// Danh sách trắng tag — rỗng ⇒ không giới hạn (M24).
    pub allowed_tool_tags: Vec<String>,
}

/// Kênh web (`[web]`) — chỉ những thứ người vận hành cần biết, không có secret.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WebChannelDto {
    /// Bật hay tắt.
    pub enabled: bool,
    /// Địa chỉ bind (`127.0.0.1:7878` = chỉ máy chủ).
    pub bind: String,
    /// Cho phép bind ngoài loopback — khi `true` phải đặt sau reverse proxy TLS.
    pub allow_remote: bool,
    /// TTL của phiên đăng nhập (giờ).
    pub session_ttl_hours: u32,
}

/// Kênh Telegram (`[telegram]`) — **không bao giờ** chứa token.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TelegramChannelDto {
    /// Bật hay tắt.
    pub enabled: bool,
    /// Số user id trong allowlist (id cụ thể không cần cho màn trạng thái).
    pub allowed_users: usize,
    /// Giới hạn số tin mỗi phút cho mỗi chat.
    pub rate_limit_per_minute: u32,
}

/// Response của `GET /api/system` — cấu hình **đã khử secret** để hiển thị.
///
/// Bất biến: endpoint này **không** trả API key, bot token, MCP `env`, hay đường dẫn
/// `bean.toml`. `api_key_env` là **tên biến môi trường**, không phải giá trị (mục 15.6).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SystemResponse {
    /// Tên agent (`agent.agent_name`).
    pub agent_name: String,
    /// Thư mục làm việc — mọi thao tác file bị jail trong đây.
    pub workspace: String,
    /// Múi giờ IANA dùng cho cron và lịch hẹn.
    pub timezone: String,
    /// Provider LLM (`anthropic` | `openai_compat`).
    pub provider: String,
    /// Model đang dùng.
    pub model: String,
    /// Tên biến môi trường chứa API key (**không** phải key).
    pub api_key_env: String,
    /// Trần token mỗi lượt gọi.
    pub max_tokens: u32,
    /// Ngân sách token cho context gửi model.
    pub context_budget_tokens: u32,
    /// Trần số bước của một run.
    pub max_steps: u32,
    /// Trần thời gian cho mỗi tool call (giây).
    pub tool_timeout_seconds: u64,
    /// Nhóm tool đang bật (`[tools].enabled`).
    pub tool_groups: Vec<String>,
    /// Project profile; rỗng ⇒ chỉ có project `default`.
    pub projects: Vec<String>,
    /// Bảng role RBAC.
    pub roles: Vec<RoleDto>,
    /// RBAC có bật không.
    pub rbac_enabled: bool,
    /// Sandbox cho `run_shell`.
    pub sandbox: SandboxDto,
    /// Kênh web.
    pub web: WebChannelDto,
    /// Kênh Telegram.
    pub telegram: TelegramChannelDto,
    /// Learning loop (đề xuất skill nháp sau run) có bật không.
    pub learning_enabled: bool,
    /// Bean có đang làm MCP server cho client ngoài không (M25).
    pub mcp_server_enabled: bool,
    /// Số MCP client được phép gọi vào Bean (M25).
    pub mcp_clients: usize,
    /// Tool browser (M26) có bật không.
    pub browser_enabled: bool,
}
