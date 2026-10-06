//! Handler REST: audit log va danh sach tool.
//!
//! # Chỉ đọc
//!
//! Audit log là bằng chứng an toàn: API này **chỉ đọc**, không có endpoint sửa/xoá. Tool
//! list lọc theo quyen RBAC cua user dang goi, khong phai tool nao cung ton tai.

use axum::Json;
use axum::extract::{Query, State};
use axum_extra::extract::cookie::CookieJar;
use bean_types::Risk;

use crate::api_types::{
    AuditListResponse, AuditQuery, McpServerDto, McpServerListResponse, RiskDto, ToolDto,
    ToolListResponse,
};

use super::{ApiResult, WebState, require_user};
pub(super) async fn list_audit(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<AuditListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let Some(audit) = state.audit.as_ref() else {
        return Ok(Json(AuditListResponse {
            entries: Vec::new(),
        }));
    };
    let entries = audit
        .read_recent(query.before, query.limit.unwrap_or(100).min(500))
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();
    Ok(Json(AuditListResponse { entries }))
}

// ────────────────── Tools / MCP / Usage / System (read-only) ──────────────────
//
// Bốn endpoint này tồn tại vì các màn Hub/Tools/MCP/Status của UI cần số liệu
// **thật**: trước đây "Tools" và "MCP" chỉ tồn tại trong `bean.toml`, còn token
// theo ngày chỉ có đúng một điểm dữ liệu (hôm nay) qua `/api/status`.

/// Map `Risk` → `RiskDto` cho các endpoint read-only mới.
const fn risk_dto(risk: Risk) -> RiskDto {
    match risk {
        Risk::Safe => RiskDto::Safe,
        Risk::Confirm => RiskDto::Confirm,
        Risk::Dangerous => RiskDto::Dangerous,
    }
}

/// Nguồn gốc tool từ tên đã public hoá: `mcp__<server>__<tool>` ⇒ MCP server.
///
/// Không có metadata "nguồn" trên `Tool`, nhưng tiền tố `mcp__<server>__` là
/// **bất biến do chính `McpRuntime` đặt ra** (mục 16), nên suy ra từ tên là đủ
/// chính xác và không phải đổi trait.
fn tool_origin(name: &str) -> (&'static str, Option<String>) {
    match name
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    {
        Some((server, _)) => ("mcp", Some(server.to_string())),
        None => ("builtin", None),
    }
}

/// `GET /api/tools` — danh sách tool người gọi **được phép thấy**.
///
/// Lọc theo RBAC xảy ra **trước** khi serialise, cùng nguyên tắc với [`list_agents`]:
/// tool bị chặn không xuất hiện trong payload, chứ không phải "UI tự ẩn".
pub(super) async fn list_tools(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<ToolListResponse>> {
    let user_id = require_user(&state, &jar).await?;
    let permissions = state.config.permissions_for(&user_id);
    let registry = state.router.registry();
    let total = registry.len();
    let tools = registry
        .names()
        .into_iter()
        .filter(|name| registry.allows(name, &permissions))
        .filter_map(|name| registry.get(&name))
        .map(|tool| {
            let spec = tool.spec();
            let (source, mcp_server) = tool_origin(&spec.name);
            ToolDto {
                name: spec.name,
                description: spec.description,
                // Tham số rỗng: cùng chuẩn với `mcp_server::gate::expose_gate`, để
                // màn Tools không tự đưa ra một định nghĩa "rủi ro" riêng.
                risk: risk_dto(tool.risk(&serde_json::json!({}))),
                source: source.to_string(),
                mcp_server,
                required_tags: {
                    let access = tool.access();
                    access
                        .required_tags
                        .iter()
                        .map(|tag| String::from(*tag))
                        .collect()
                },
                extra_tags: tool
                    .access()
                    .also_visible_to
                    .iter()
                    .map(|tag| String::from(*tag))
                    .collect(),
                untrusted: tool.marks_untrusted(),
            }
        })
        .collect();
    Ok(Json(ToolListResponse {
        tools,
        total,
        viewer_role: permissions.role,
        rbac_enabled: state.config.rbac_enabled(),
    }))
}

/// `GET /api/mcp` — server đã khai báo + trạng thái **suy ra từ registry**.
///
/// `connected` nghĩa là "server đã initialize + discovery thành công, tool của nó
/// đang nằm trong registry" — không phải health check sống/chết, UI phải nói đúng
/// như vậy. Lỗi lúc khởi động đã có log cảnh báo ở `McpRuntime`.
///
/// `env` của server **không** được trả: giá trị là secret (mục 15.6), tên biến cũng
/// gợi ý loại credential đang dùng.
pub(super) async fn list_mcp_servers(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<McpServerListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let names = state.router.registry().names();
    let servers = state
        .config
        .mcp_servers
        .iter()
        .map(|server| {
            let prefix = format!("mcp__{}__", server.name);
            let tool_count = names
                .iter()
                .filter(|name| name.starts_with(prefix.as_str()))
                .count();
            McpServerDto {
                name: server.name.clone(),
                command: server.command.clone(),
                args: server.args.clone(),
                trusted: server.trust,
                call_timeout_seconds: server.call_timeout_seconds,
                required_tags: server.tool_tags.clone(),
                tool_count,
                connected: tool_count > 0,
            }
        })
        .collect();
    Ok(Json(McpServerListResponse { servers }))
}
