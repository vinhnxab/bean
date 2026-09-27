//! Cổng expose cứng của MCP server (M25).
//!
//! # Vì sao cần một cổng riêng, không dựa vào RBAC
//!
//! `RolePermissions::allows` trả `true` cho **mọi** tool khi role giữ tag `"*"`
//! (`admin`). Nếu chỉ dựa vào RBAC thì một client MCP bị lộ token mà được map vào role
//! `admin` sẽ thấy — và gọi — `write_file`, `run_shell`, `security_scan`,
//! `marketing_publish`. Điều đó vi phạm trực tiếp phạm vi cứng của M25: *"TUYỆT ĐỐI
//! không expose bất kỳ tool nào có thể ghi/thực thi … kể cả nếu client tự xưng có quyền
//! cao"*.
//!
//! Vì vậy lọc ở đây là **deny-by-default**: tool chỉ qua được khi mang đúng một tag
//! trong [`MCP_EXPOSED_TAGS`] **và** [`Risk::Safe`]. Đây là cổng *thắt* trên RBAC, không
//! thay thế nó.

use beanagent_tools::ToolRegistry;
use beanagent_types::config::{BILLING_TAG, INFRA_READ_TAG, MEMORY_READ_TAG};
use beanagent_types::{Risk, RolePermissions, ToolSpec};

/// Ba tag duy nhất được expose qua MCP server (M25, phạm vi cứng).
///
/// * `infra-read` — đọc log/CVE/uptime (M22).
/// * `billing-read` — đọc chi phí cloud (M22a).
/// * `memory-read` — đọc `MEMORY.md`/`USER.md` (M25).
///
/// Mọi tag khác — kể cả tag do `[[mcp_servers]].tool_tags` sinh ra — bị loại.
pub const MCP_EXPOSED_TAGS: [&str; 3] = [INFRA_READ_TAG, BILLING_TAG, MEMORY_READ_TAG];

/// Kết quả xét của cổng expose cho một tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposeDecision {
    /// Được expose (đã qua cả cổng tag lẫn kiểm tra `Safe`).
    Allow,
    /// Tool mang tag được expose nhưng **không** phải `Safe` — không có chỗ nào để hỏi
    /// xác nhận, nên không thể chạy vô hại.
    NotSafe {
        /// Mức rủi ro thật của tool.
        risk: Risk,
    },
    /// Tool không mang tag nào trong [`MCP_EXPOSED_TAGS`].
    TagNotExposed,
    /// Tool không tồn tại trong registry.
    UnknownTool,
    /// Client không được phép gọi tool này theo RBAC của role.
    DeniedByRole,
}

/// Tool có được expose cho `perms` không? (`true` = [`ExposeDecision::Allow`]).
#[must_use]
pub fn allows(tool_name: &str, registry: &ToolRegistry, perms: &RolePermissions) -> bool {
    expose_gate(tool_name, registry, perms) == ExposeDecision::Allow
}

/// Cổng expose: kiểm tag + `Safe` + RBAC, theo thứ tự rẻ → đắt.
#[must_use]
pub fn expose_gate(
    tool_name: &str,
    registry: &ToolRegistry,
    perms: &RolePermissions,
) -> ExposeDecision {
    let Some(tool) = registry.get(tool_name) else {
        return ExposeDecision::UnknownTool;
    };
    // 1. Tag: phải nằm trong allowlist. Hỏi cả `required_tags` **và** `also_visible_to`
    //    vì M24 cho phép mở tool untagged cho một tag cụ thể — cùng một câu hỏi "tag nào
    //    mở tool này" cho cả hai cơ chế, nên phải hỏi cả hai.
    let mentioned = tool
        .required_tags()
        .into_iter()
        .chain(tool.also_visible_to())
        .any(|tag| MCP_EXPOSED_TAGS.contains(&tag));
    if !mentioned {
        return ExposeDecision::TagNotExposed;
    }
    // 2. Rủi ro: chỉ `Safe`. `Confirm`/`Dangerous` cần người dùng bấm nút, mà client
    //    MCP không có kênh nào để hỏi ⇒ coi như không dùng được, nên chặn hẳn.
    //    `risk` nhận `args` nên tham số rỗng là đại diện an toàn (tool nào đổi rủi ro
    //    theo tham số đều không nên expose).
    let risk = tool.risk(&serde_json::json!({}));
    if risk != Risk::Safe {
        return ExposeDecision::NotSafe { risk };
    }
    // 3. RBAC: đúng hàm `allows` của M21, không viết lại.
    if !registry.allows(tool_name, perms) {
        return ExposeDecision::DeniedByRole;
    }
    ExposeDecision::Allow
}

/// Danh sách [`ToolSpec`] client được thấy, **đã sort theo tên** (ổn định giữa các
/// lần gọi `tools/list`).
#[must_use]
pub fn visible_specs(registry: &ToolRegistry, perms: &RolePermissions) -> Vec<ToolSpec> {
    registry
        .names()
        .into_iter()
        .filter(|name| allows(name, registry, perms))
        .filter_map(|name| registry.get(&name).map(|tool| tool.spec()))
        .collect()
}

/// Tương tự [`visible_specs`] nhưng chỉ trả **tên** tool — tiện cho log và test.
#[must_use]
pub fn visible_names(registry: &ToolRegistry, perms: &RolePermissions) -> Vec<String> {
    registry
        .names()
        .into_iter()
        .filter(|name| allows(name, registry, perms))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeSet;
    use std::sync::Arc;

    use async_trait::async_trait;
    use beanagent_tools::{Tool, ToolCtx, ToolError, ToolRegistry};
    use beanagent_types::{Risk, RolePermissions, ToolSpec};

    use super::*;

    struct Stub {
        name: &'static str,
        risk: Risk,
        tags: Vec<&'static str>,
    }

    #[async_trait]
    impl Tool for Stub {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(self.name, "stub", serde_json::json!({"type": "object"}))
        }

        fn risk(&self, _args: &serde_json::Value) -> Risk {
            self.risk
        }

        fn required_tags(&self) -> Vec<&str> {
            self.tags.clone()
        }

        async fn call(
            &self,
            _ctx: &ToolCtx,
            _args: serde_json::Value,
        ) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }

    fn registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        for (name, risk, tags) in [
            ("billing_read_cost", Risk::Safe, vec![BILLING_TAG]),
            ("infra_read_logs", Risk::Safe, vec![INFRA_READ_TAG]),
            ("dev_write", Risk::Confirm, vec!["dev-write"]),
            ("shell_confirm", Risk::Confirm, vec![INFRA_READ_TAG]),
            ("untrusted_read", Risk::Safe, vec![]),
        ] {
            reg.register(Arc::new(Stub { name, risk, tags })).unwrap();
        }
        reg
    }

    fn perms(tags: &[&str]) -> RolePermissions {
        RolePermissions::restricted_to(
            "test",
            tags.iter()
                .map(|t| (*t).to_string())
                .collect::<BTreeSet<_>>(),
            BTreeSet::new(),
        )
    }

    #[test]
    fn admin_wildcard_still_cannot_reach_write_tools() {
        let reg = registry();
        // `*` là mọi quyền theo RBAC — nhưng cổng expose phải chặn vẫn.
        let admin = RolePermissions::unrestricted("admin");
        assert!(allows("billing_read_cost", &reg, &admin));
        assert_eq!(
            expose_gate("dev_write", &reg, &admin),
            ExposeDecision::TagNotExposed
        );
    }

    #[test]
    fn exposed_tag_with_non_safe_risk_is_rejected() {
        let reg = registry();
        let p = perms(&[INFRA_READ_TAG]);
        assert_eq!(
            expose_gate("shell_confirm", &reg, &p),
            ExposeDecision::NotSafe {
                risk: Risk::Confirm
            }
        );
    }

    #[test]
    fn role_without_the_tag_is_denied_even_for_exposed_tag() {
        let reg = registry();
        // Role chỉ có `billing-read` ⇒ `infra_read_logs` (Safe, đúng tag) bị chặn ở
        // **tầng RBAC**, không phải vì tag sai. Đây là chốt chặn thứ hai bên cạnh cổng
        // expose: client có thể hợp lệ (token đúng) mà vẫn không đủ quyền.
        let p = perms(&[BILLING_TAG]);
        assert!(allows("billing_read_cost", &reg, &p));
        assert_eq!(
            expose_gate("infra_read_logs", &reg, &p),
            ExposeDecision::DeniedByRole
        );
    }

    #[test]
    fn no_access_role_sees_nothing_even_for_exposed_tags() {
        let reg = registry();
        let none = RolePermissions::deny_all(beanagent_types::NO_ACCESS_ROLE);
        for name in ["billing_read_cost", "infra_read_logs"] {
            assert_eq!(
                expose_gate(name, &reg, &none),
                ExposeDecision::DeniedByRole,
                "{name}"
            );
        }
    }

    #[test]
    fn unknown_and_untagged_tools_are_not_exposed() {
        let reg = registry();
        let p = perms(&[BILLING_TAG]);
        assert_eq!(
            expose_gate("khong_ton_tai", &reg, &p),
            ExposeDecision::UnknownTool
        );
        assert_eq!(
            expose_gate("untrusted_read", &reg, &p),
            ExposeDecision::TagNotExposed
        );
    }

    #[test]
    fn visible_specs_are_sorted_and_filtered() {
        let reg = registry();
        let specs = visible_specs(&reg, &perms(&[BILLING_TAG]));
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["billing_read_cost"]);
    }
}
