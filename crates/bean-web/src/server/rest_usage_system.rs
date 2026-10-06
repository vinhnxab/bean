//! Handler REST: lịch sử token và thông tin hệ thống.
//!
//! # Không lộ secret
//!
//! `system_info` phải fail-closed: chi dung `config` đã validate, tuyệt đối không đọc
//! token. Test `system_endpoint_reports_config_without_secrets` canh điều này.

use axum::Json;
use axum::extract::{Query, State};
use axum_extra::extract::cookie::CookieJar;

use crate::api_types::{
    RoleDto, SandboxDto, SystemResponse, TelegramChannelDto, UsageDayDto, UsageListResponse,
    UsageQuery, WebChannelDto,
};

use super::{ApiFailure, ApiResult, WebState, require_user};
/// Số ngày mặc định và trần của `GET /api/usage`.
const DEFAULT_USAGE_DAYS: i64 = 14;
/// Trần cứng: mỗi ngày là một query SQLite, và biểu đồ theo ngày dài hơn một tháng
/// không còn dùng để đọc xu hướng nữa.
const MAX_USAGE_DAYS: i64 = 31;

/// `GET /api/usage?days=14` — token đã dùng theo từng ngày (khoá UTC như `/status`).
pub(super) async fn usage_history(
    State(state): State<WebState>,
    jar: CookieJar,
    Query(query): Query<UsageQuery>,
) -> ApiResult<Json<UsageListResponse>> {
    let _ = require_user(&state, &jar).await?;
    let days = query
        .days
        .unwrap_or(DEFAULT_USAGE_DAYS)
        .clamp(1, MAX_USAGE_DAYS);
    // Lùi từng ngày bằng `pred_opt`: project này không dùng API trừ `TimeDelta` nào,
    // và đây là cách duy nhất không chạm API đang bị chrono đánh dấu deprecated.
    let mut dates = Vec::with_capacity(usize::try_from(days).unwrap_or(1));
    let mut cursor = Some(chrono::Utc::now().date_naive());
    for _ in 0..days {
        let Some(date) = cursor else { break };
        dates.push(date);
        cursor = date.pred_opt();
    }
    dates.reverse();
    let mut history = Vec::with_capacity(dates.len());
    for date in dates {
        let day = date.format("%Y-%m-%d").to_string();
        let usage = state
            .store
            .usage(&day)
            .await
            .map_err(|_| ApiFailure::internal())?;
        history.push(UsageDayDto {
            input_tokens: u64::from(usage.input_tokens),
            output_tokens: u64::from(usage.output_tokens),
            total_tokens: u64::from(usage.total()),
            day,
        });
    }
    let today_tokens = history.last().map_or(0, |entry| entry.total_tokens);
    Ok(Json(UsageListResponse {
        days: history,
        daily_token_budget: state.config.security.daily_token_budget,
        today_tokens,
    }))
}

/// `GET /api/system` — cấu hình **đã khử secret** cho màn Status.
///
/// Không trả: API key, bot token, MCP `env`, user id Telegram, đường dẫn `bean.toml`.
/// Trả **tên** biến môi trường (`api_key_env`) vì đó là cách cấu hình, không phải
/// bí mật (mục 15.6).
pub(super) async fn system_info(
    State(state): State<WebState>,
    jar: CookieJar,
) -> ApiResult<Json<SystemResponse>> {
    let _ = require_user(&state, &jar).await?;
    let config = &state.config;
    let projects = if config.projects.is_empty() {
        vec![bean_types::config::DEFAULT_PROJECT.to_string()]
    } else {
        config.projects.iter().map(|p| p.name.clone()).collect()
    };
    Ok(Json(SystemResponse {
        agent_name: config.agent.agent_name.clone(),
        workspace: config.agent.workspace.display().to_string(),
        timezone: config.agent.timezone.clone(),
        provider: config.llm.provider.as_str().to_string(),
        model: config.llm.model.clone(),
        api_key_env: config.llm.api_key_env.clone(),
        max_tokens: config.llm.max_tokens,
        context_budget_tokens: config.agent.context_budget_tokens,
        max_steps: config.agent.max_steps,
        tool_timeout_seconds: config.security.tool_timeout_seconds,
        tool_groups: config.tools.enabled.clone(),
        projects,
        roles: config
            .roles
            .iter()
            .map(|role| RoleDto {
                name: role.name.clone(),
                tool_tags: role.tool_tags.clone(),
                forbid_tags: role.forbid_tags.clone(),
                allowed_tool_tags: role.allowed_tool_tags.clone(),
            })
            .collect(),
        rbac_enabled: config.rbac_enabled(),
        sandbox: SandboxDto {
            mode: config.security.sandbox.mode.as_str().to_string(),
            image: config.security.sandbox.image.clone(),
            network: config.security.sandbox.network,
            memory: config.security.sandbox.memory.clone(),
            cpus: config.security.sandbox.cpus,
            pids_limit: config.security.sandbox.pids_limit,
            timeout_seconds: config.security.sandbox.timeout_seconds,
        },
        web: WebChannelDto {
            enabled: config.web.enabled,
            bind: config.web.bind.to_string(),
            allow_remote: config.web.allow_remote,
            session_ttl_hours: config.web.session_ttl_hours,
        },
        telegram: TelegramChannelDto {
            enabled: config.telegram.enabled,
            allowed_users: config.telegram.allowed_user_ids.len(),
            rate_limit_per_minute: config.telegram.rate_limit_per_minute,
        },
        learning_enabled: config.learning.enabled,
        mcp_server_enabled: config.mcp_server.enabled,
        mcp_clients: config.mcp_clients.len(),
        browser_enabled: config.browser.enabled,
    }))
}
