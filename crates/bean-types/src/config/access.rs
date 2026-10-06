//! Truy vấn cấu hình đã nạp: rbac, role, mcp client, project.

use std::path::Path;

use crate::rbac::{NO_ACCESS_ROLE, RolePermissions};

use super::*;

impl Config {
    /// RBAC có đang bật không? (M21.3)
    ///
    /// Chỉ bật khi người dùng khai báo ít nhất một `agent.user_roles`. Nếu không, mọi user
    /// trong `allowed_users` giữ hành vi cũ (thấy mọi tool) — nhờ vậy cài đặt một-người-dùng
    /// sẵn có **không** đột nhiên mất tool (D10.3).
    #[must_use]
    pub fn rbac_enabled(&self) -> bool {
        !self.agent.user_roles.is_empty()
    }
}
impl Config {
    /// **Điểm quyết định RBAC duy nhất** (M21.3): `user_id` → [`RolePermissions`].
    ///
    /// Router gọi hàm này **một lần** mỗi run rồi truyền struct kết quả (tuần tự hoá được)
    /// xuống agent loop; agent loop không tự tra cứu role ⇒ không có logic RBAC rải rác.
    ///
    /// * RBAC tắt ⇒ [`RolePermissions::unrestricted`] (giữ hành vi cũ).
    /// * RBAC bật mà user có trong `user_roles` ⇒ tag của role đó.
    /// * RBAC bật mà user **không** có trong map ⇒ `no-access` (deny-all).
    /// * `user_roles` trỏ tới role không tồn tại ⇒ `no-access`; `validate()` đã chặn từ lúc
    ///   nạp config nên nhánh này chỉ là lưới an toàn.
    #[must_use]
    pub fn permissions_for(&self, user_id: &str) -> RolePermissions {
        if !self.rbac_enabled() {
            return RolePermissions::unrestricted("default");
        }
        let Some(role_name) = self.agent.user_roles.get(user_id) else {
            tracing::warn!(
                user_id,
                "user không có trong agent.user_roles — dùng role no-access (không thấy tool nào)"
            );
            return RolePermissions::deny_all(NO_ACCESS_ROLE);
        };
        match self.role(role_name) {
            Some(role) => {
                RolePermissions::restricted_to(&role.name, role.tag_set(), role.allowed_tag_set())
            }
            None => {
                tracing::error!(user_id, role = %role_name,
                    "user_roles trỏ tới role không tồn tại — dùng no-access");
                RolePermissions::deny_all(NO_ACCESS_ROLE)
            }
        }
    }
}
impl Config {
    /// Tra role theo tên trong `[[roles]]`.
    #[must_use]
    pub fn role(&self, name: &str) -> Option<&RoleConfig> {
        self.roles.iter().find(|role| role.name == name)
    }
}
impl Config {
    /// Tra client MCP theo tên trong `[[mcp_clients]]`.
    #[must_use]
    pub fn mcp_client(&self, name: &str) -> Option<&McpClientConfig> {
        self.mcp_clients
            .iter()
            .find(|client| client.name.trim() == name)
    }
}
impl Config {
    /// Identity đầy đủ của một client MCP (M25): `mcp-client:<name>`.
    #[must_use]
    pub fn mcp_client_identity(name: &str) -> String {
        format!("{MCP_CLIENT_PREFIX}{name}")
    }
}
impl Config {
    /// Ngân sách context token **riêng cho role** (M21.7).
    #[must_use]
    pub fn context_budget_for(&self, perms: &RolePermissions) -> u32 {
        self.role(&perms.role)
            .and_then(|role| role.context_budget_tokens)
            .unwrap_or(self.agent.context_budget_tokens)
    }
}
impl Config {
    /// Ngân sách token/ngày **riêng cho role** (M21.7).
    #[must_use]
    pub fn daily_budget_for(&self, perms: &RolePermissions) -> u64 {
        self.role(&perms.role)
            .and_then(|role| role.daily_token_budget)
            .unwrap_or(self.security.daily_token_budget)
    }
}
impl Config {
    /// Danh sách project đã khai báo; luôn có ít nhất project `default`.
    ///
    /// `default` ánh xạ tới `agent.workspace` (M21.1) — Bean tự phát triển chính nó là một
    /// project profile **không đặc quyền hơn** project nào khác: cùng cơ chế, cùng jail.
    #[must_use]
    pub fn project_profiles(&self) -> Vec<ProjectConfig> {
        let mut profiles = vec![ProjectConfig {
            name: DEFAULT_PROJECT.to_string(),
            workspace: self.agent.workspace.clone(),
        }];
        for project in &self.projects {
            if project.name != DEFAULT_PROJECT {
                profiles.push(project.clone());
            }
        }
        profiles
    }
}
impl Config {
    /// Workspace của một project; `None` nếu tên không tồn tại.
    #[must_use]
    pub fn project_workspace(&self, name: &str) -> Option<&Path> {
        if name == DEFAULT_PROJECT {
            return Some(&self.agent.workspace);
        }
        self.projects
            .iter()
            .find(|project| project.name == name)
            .map(|project| project.workspace.as_path())
    }
}
