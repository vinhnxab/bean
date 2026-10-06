//! Kiểm tra cấu hình lõi: rbac, projects, paths, web/kênh + validator dùng chung.

use std::collections::BTreeSet;

use crate::rbac::{NO_ACCESS_ROLE, WILDCARD_TAG};

use super::error::expand_tilde;
use super::error::invalid;
use super::validate_integrations::is_valid_mcp_name;
use super::*;

impl Config {
    /// Kiểm tra bảng `[[roles]]` và `agent.user_roles` (M21.2, M21.6).
    pub(super) fn validate_rbac(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for role in &self.roles {
            let name = role.name.trim();
            if name.is_empty() {
                return Err(invalid("[[roles]] có role thiếu tên"));
            }
            if name == NO_ACCESS_ROLE {
                return Err(invalid(format!(
                    "`{NO_ACCESS_ROLE}` là role dự phòng của hệ thống (deny-all); không được khai báo lại"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!("[[roles]] có role trùng tên `{name}`")));
            }
            // (M21.6) Nguyên tắc four-eyes kiểm ở tầng code: cấp vừa cấm là lỗi cấu hình,
            // không phải việc để model tự phát hiện lúc chạy.
            let granted = role.tag_set();
            for forbidden in &role.forbid_tags {
                let forbidden = forbidden.trim();
                if granted.contains(forbidden) {
                    return Err(invalid(format!(
                        "role `{name}` vừa được cấp tag `{forbidden}` vừa khai báo nó trong forbid_tags — vi phạm nguyên tắc four-eyes"
                    )));
                }
            }
            if role
                .context_budget_tokens
                .is_some_and(|value| value < 1_000)
            {
                return Err(invalid(format!(
                    "role `{name}`.context_budget_tokens tối thiểu 1000"
                )));
            }
            // (M24) Danh sách trắng phải chứa ít nhất một tag mà role thực sự được cấp,
            // nếu không thì role đó bị chặn khỏi MỌI tool (kể cả untagged) mà không ai
            // hỏi — cấu hình im lặng, rất dễ quên.
            let allowed = role.allowed_tag_set();
            if !allowed.is_empty() && !allowed.iter().any(|tag| granted.contains(tag)) {
                return Err(invalid(format!(
                    "role `{name}` khai allowed_tool_tags nhưng tool_tags không chứa tag nào \
                     trong đó — role sẽ không thấy tool nào"
                )));
            }
            // Wildcard `*` trong danh sách trắng là mâu thuẫn: nó đã là "mọi quyền" ở
            // `tool_tags`, thêm vào đây chỉ làm mờ ý nghĩa.
            if allowed.iter().any(|tag| tag == WILDCARD_TAG) {
                return Err(invalid(format!(
                    "role `{name}`: không cần `{WILDCARD_TAG}` trong allowed_tool_tags — \
                     tag đó đã bỏ qua danh sách trắng; bỏ hẳn trường này nếu không giới hạn"
                )));
            }
        }
        for (user, role) in &self.agent.user_roles {
            if self.role(role).is_none() {
                return Err(invalid(format!(
                    "agent.user_roles[`{user}`] trỏ tới role `{role}` không tồn tại trong [[roles]]"
                )));
            }
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[[projects]]` (M21.1).
    pub(super) fn validate_projects(&self) -> Result<(), ConfigError> {
        let mut seen = BTreeSet::new();
        for project in &self.projects {
            let name = project.name.trim();
            if name.is_empty() {
                return Err(invalid("[[projects]] có project thiếu tên"));
            }
            if name == DEFAULT_PROJECT {
                return Err(invalid(format!(
                    "`{DEFAULT_PROJECT}` là project dự phòng ánh xạ tới agent.workspace; không khai báo lại"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(invalid(format!(
                    "[[projects]] có project trùng tên `{name}`"
                )));
            }
        }
        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[agent]`, `[tools]`, `[llm]`, `[security]`.
    pub(super) fn validate_core(&mut self) -> Result<(), ConfigError> {
        if !(1..=1000).contains(&self.agent.max_steps) {
            return Err(invalid(format!(
                "agent.max_steps phải trong 1..=1000 (đang là {})",
                self.agent.max_steps
            )));
        }
        if self.agent.context_budget_tokens < 1_000 {
            return Err(invalid(format!(
                "agent.context_budget_tokens quá nhỏ ({}); tối thiểu 1000",
                self.agent.context_budget_tokens
            )));
        }
        if self.agent.timezone.trim().is_empty() {
            return Err(invalid(
                "agent.timezone rỗng — cần tên múi giờ IANA, ví dụ `Asia/Ho_Chi_Minh`",
            ));
        }
        if self.agent.allowed_users.is_empty() {
            return Err(invalid(
                "agent.allowed_users rỗng — sẽ không ai dùng được agent",
            ));
        }

        for group in &self.tools.enabled {
            if !KNOWN_TOOL_GROUPS.contains(&group.as_str()) {
                return Err(invalid(format!(
                    "tools.enabled chứa `{group}` không hợp lệ; chỉ nhận: {}",
                    KNOWN_TOOL_GROUPS.join(", ")
                )));
            }
        }

        let web_enabled = self.tools.enabled.iter().any(|group| group == "web");
        let web_search = &self.tools.web_search;
        if web_enabled
            && web_search.provider.requires_api_key()
            && web_search.api_key_env.trim().is_empty()
        {
            return Err(invalid(
                "tools.web_search.api_key_env rỗng — cần TÊN biến môi trường chứa API key",
            ));
        }
        if web_enabled && let Some(base_url) = web_search.base_url.as_deref() {
            validate_http_url("tools.web_search.base_url", base_url)?;
        }
        if web_enabled
            && web_search.provider == WebSearchProvider::Searxng
            && web_search.base_url.is_none()
        {
            return Err(invalid(
                "tools.web_search.provider = searxng cần tools.web_search.base_url",
            ));
        }
        if self.learning.min_tool_calls == 0 {
            return Err(invalid("learning.min_tool_calls phải > 0"));
        }
        if self.learning.proposal_interval_minutes == 0 {
            return Err(invalid("learning.proposal_interval_minutes phải > 0"));
        }

        if self.llm.model.trim().is_empty() {
            return Err(invalid("llm.model rỗng"));
        }
        if !self
            .llm
            .effective_allowed_models()
            .iter()
            .any(|m| m == &self.llm.model)
        {
            return Err(invalid(format!(
                "llm.model `{}` không nằm trong llm.allowed_models",
                self.llm.model
            )));
        }
        if self.llm.api_key_env.trim().is_empty() {
            return Err(invalid(
                "llm.api_key_env rỗng — cần TÊN biến môi trường chứa API key",
            ));
        }
        if !(1..=1_000_000).contains(&self.llm.max_tokens) {
            return Err(invalid(format!(
                "llm.max_tokens phải trong 1..=1000000 (đang là {})",
                self.llm.max_tokens
            )));
        }
        if let Some(base_url) = self.llm.base_url.as_deref() {
            validate_http_url("llm.base_url", base_url)?;
        }

        if self.security.daily_token_budget == 0 {
            return Err(invalid("security.daily_token_budget phải > 0"));
        }
        let sandbox = &self.security.sandbox;
        if sandbox.image.trim().is_empty() {
            return Err(invalid("security.sandbox.image rỗng"));
        }
        if sandbox.memory.trim().is_empty() {
            return Err(invalid("security.sandbox.memory rỗng — ví dụ `512m`"));
        }
        // NaN cũng phải bị từ chối (vì vậy không dùng `!(cpus > 0.0)` — clippy::neg_cmp_op_on_partial_ord).
        if sandbox.cpus <= 0.0 || !sandbox.cpus.is_finite() {
            return Err(invalid(format!(
                "security.sandbox.cpus phải là số hữu hạn > 0 (đang là {})",
                sandbox.cpus
            )));
        }
        if sandbox.timeout_seconds == 0 {
            return Err(invalid("security.sandbox.timeout_seconds phải > 0"));
        }
        if sandbox.mode == SandboxMode::Host {
            tracing::warn!(
                "security.sandbox.mode = \"host\": mọi lệnh shell sẽ bị coi là Dangerous (agents.md mục 15.2)"
            );
        }

        Ok(())
    }
}
impl Config {
    /// Kiểm tra `[web]`, `[telegram]`, `[[mcp_servers]]`.
    pub(super) fn validate_web_and_channels(&mut self) -> Result<(), ConfigError> {
        let origin = self.web.public_origin.trim_end_matches('/').to_string();
        validate_http_url("web.public_origin", &origin)?;
        self.web.public_origin = origin;

        if !self.web.bind.ip().is_loopback() && !self.web.allow_remote {
            return Err(invalid(format!(
                "web.bind = `{}` không phải loopback nhưng web.allow_remote = false (agents.md mục 15.7)",
                self.web.bind
            )));
        }
        if self.web.bind.ip().is_loopback() && self.web.allow_remote {
            tracing::warn!(
                "web.allow_remote = true nhưng web.bind vẫn là loopback — kiểm tra lại reverse proxy/Tailscale"
            );
        }
        if self.web.session_ttl_hours == 0 {
            return Err(invalid("web.session_ttl_hours phải > 0"));
        }
        if self.web.trust_proxy {
            tracing::warn!(
                "web.trust_proxy = true: chỉ dùng khi Bean nằm sau reverse proxy tin cậy (D4.3)"
            );
        }

        if self.telegram.enabled {
            if self.telegram.allowed_user_ids.is_empty() {
                return Err(invalid(
                    "telegram.enabled = true nhưng telegram.allowed_user_ids rỗng — allowlist là bắt buộc (mục 13)",
                ));
            }
            for user_id in &self.telegram.allowed_user_ids {
                let identity = format!("telegram:{user_id}");
                if !self
                    .agent
                    .allowed_users
                    .iter()
                    .any(|allowed| allowed == &identity)
                {
                    return Err(invalid(format!(
                        "telegram user {user_id} phải có `{identity}` trong agent.allowed_users để Router cho phép"
                    )));
                }
            }
            if self.telegram.token_env.trim().is_empty() {
                return Err(invalid(
                    "telegram.token_env rỗng — cần TÊN biến môi trường chứa bot token",
                ));
            }
            if self.telegram.rate_limit_per_minute == 0 {
                return Err(invalid("telegram.rate_limit_per_minute phải > 0"));
            }
        }

        let mut names = BTreeSet::new();
        for server in &self.mcp_servers {
            if !is_valid_mcp_name(&server.name) {
                return Err(invalid(format!(
                    "mcp_servers[].name `{}` không hợp lệ — chỉ dùng chữ, số, `_`, `-` (vì trở thành tiền tố tên tool)",
                    server.name
                )));
            }
            if !names.insert(server.name.as_str()) {
                return Err(invalid(format!(
                    "mcp_servers có tên trùng: `{}`",
                    server.name
                )));
            }
            if server.command.trim().is_empty() {
                return Err(invalid(format!(
                    "mcp_servers `{}`: command rỗng",
                    server.name
                )));
            }
        }

        Ok(())
    }
}
impl Config {
    /// Chuẩn hoá đường dẫn (`~` → `$HOME`) sau cùng, để các bước kiểm tra phía trên
    /// luôn so sánh trên giá trị người dùng đã viết.
    pub(super) fn validate_paths(&mut self) -> Result<(), ConfigError> {
        if self.data.dir.as_os_str().is_empty() {
            return Err(invalid("data.dir rỗng"));
        }
        if self.agent.workspace.as_os_str().is_empty() {
            return Err(invalid("agent.workspace rỗng"));
        }
        self.agent.workspace = expand_tilde(&self.agent.workspace)?;
        self.data.dir = expand_tilde(&self.data.dir)?;
        Ok(())
    }
}
/// Kiểm tra cú pháp một giá trị trong `[[infra_scope]]`.
///
/// Trả `Err(mô tả lỗi)` nếu sai. Cố ý **không** chấp nhận hostname: xem
/// [`ScanTargetKind`].
pub fn validate_scope_value(kind: ScanTargetKind, value: &str) -> Result<(), &'static str> {
    use std::net::IpAddr;
    match kind {
        ScanTargetKind::Ip => match value.parse::<IpAddr>() {
            Ok(_) => Ok(()),
            Err(_) => Err("không phải địa chỉ IP hợp lệ"),
        },
        ScanTargetKind::Cidr => {
            let Some((addr, len)) = value.split_once('/') else {
                return Err("thiếu hậu tố /len");
            };
            let Ok(ip) = addr.parse::<IpAddr>() else {
                return Err("phần địa chỉ không hợp lệ");
            };
            let Ok(len) = len.parse::<u8>() else {
                return Err("độ dài tiền tố /len không phải số");
            };
            let max = if ip.is_ipv4() { 32 } else { 128 };
            if len > max {
                return Err("độ dài tiền tố vượt giới hạn của họ địa chỉ");
            }
            Ok(())
        }
    }
}

/// Kiểm tra một mẫu trong `[browser].allowed_origins` (M26).
///
/// Cùng luật với [`bean-browser`](crate::config) nhưng **viết lại ở đây**,
/// không import từ crate browser: chiều phụ thuộc phải là `types` → `browser`, không
/// bao giờ ngược lại (nếu không, mọi thứ dùng `Config` cũng phải kéo `chromiumoxide`
/// vào). Hai bản phải giữ cùng quy tắc — test ở `bean-browser` chốt hành vi thật.
pub(super) fn validate_origin_pattern(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("mẫu rỗng".into());
    }
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err("thiếu scheme, phải có dạng `scheme://host[:port]`".into());
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return Err(format!("scheme `{scheme}` không được phép"));
    }
    if rest.contains('/') {
        return Err("origin không được chứa đường dẫn".into());
    }
    if rest.contains('@') {
        return Err("origin không được chứa credential".into());
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) => {
            let parsed: u16 = port
                .parse()
                .map_err(|_| format!("port `{port}` không phải số"))?;
            if parsed == 0 {
                return Err("port 0 không hợp lệ".into());
            }
            (host, parsed)
        }
        None => (rest, 0),
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return Err("thiếu host".into());
    }
    if host.starts_with('*') && !host.starts_with("*.") {
        return Err("wildcard chỉ được dùng dạng `*.host`".into());
    }
    if host.contains('*') && !host.starts_with("*.") {
        return Err(format!(
            "wildcard chỉ được dùng ở label đầu tiên, không phải `{host}`"
        ));
    }
    if host.matches('*').count() > 1 {
        return Err("chỉ nhận tối đa một wildcard".into());
    }
    if host.trim_start_matches("*.").is_empty() {
        return Err("mẫu wildcard `*` trần quá rộng; hãy ghi domain cụ thể".into());
    }
    let _ = port;
    if !host.starts_with('*') {
        let bare = host.trim_matches(|c| c == '[' || c == ']');
        let looks_like_ip = bare.parse::<std::net::IpAddr>().is_ok();
        if !looks_like_ip
            && !bare.split('.').all(|label| {
                !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        {
            return Err(format!("host `{host}` không phải tên miền hợp lệ"));
        }
    }
    Ok(())
}

/// Kiểm tra một tên biến trong `[[mcp_servers]].inherit_env`.
///
/// `Command::env` **panic** khi tên rỗng hoặc chứa `=`/NUL, và im lặng bỏ qua tên có
/// ký tự lạ trên một số nền tảng. Vì đây là dữ liệu từ file cấu hình của người dùng,
/// phải chết sớm ở tầng config với thông điệp chỉ đường thay vì sập tiến trình.
///
/// Chỉ chấp nhận `[A-Za-z_][A-Za-z0-9_]*` — đúng POSIX, và là tập con của những gì
/// mọi hệ điều hành Bean chạy trên chấp nhận cho tên biến.
pub(super) fn validate_inherited_env_name(server: &str, variable: &str) -> Result<(), ConfigError> {
    let name = variable.trim();
    if name.is_empty() {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên biến rỗng"
        )));
    }
    if name.contains('=') || name.contains('\0') {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên `{name}` chứa '=' hoặc NUL — không phải tên biến hợp lệ"
        )));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên `{name}` chứa ký tự ngoài [A-Za-z0-9_]"
        )));
    }
    if name.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return Err(invalid(format!(
            "[[mcp_servers]].inherit_env của `{server}` có tên `{name}` bắt đầu bằng chữ số"
        )));
    }
    Ok(())
}

/// Kiểm tra URL `http(s)` ở mức tối thiểu (M1 chưa cần crate `url`).
pub(super) fn validate_http_url(field: &str, value: &str) -> Result<(), ConfigError> {
    let lowered = value.to_ascii_lowercase();
    let rest = lowered
        .strip_prefix("https://")
        .or_else(|| lowered.strip_prefix("http://"))
        .ok_or_else(|| {
            invalid(format!(
                "{field} phải bắt đầu bằng `http://` hoặc `https://` (đang là `{value}`)"
            ))
        })?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(invalid(format!("{field} thiếu host (đang là `{value}`)")));
    }
    Ok(())
}
