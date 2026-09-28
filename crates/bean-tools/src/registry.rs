//! `ToolRegistry` — sổ đăng ký tool cho một tiến trình agent (agents.md mục 7.1).

use std::collections::BTreeMap;
use std::sync::Arc;

use bean_types::{RolePermissions, ToolSpec};

use crate::error::ToolError;
use crate::tool::Tool;
use crate::workspace::WorkspaceFs;

/// Registry lưu `Arc<dyn Tool>` theo tên; `specs()` trả về **đã sort theo tên** để
/// prompt/schema gửi model ổn định giữa các lượt (không nhảy thứ tự ngẫu nhiên).
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    /// Workspace dùng cho tool (M4: `CapWorkspace` trên cap-std — bean-security).
    workspace: Option<Arc<dyn WorkspaceFs>>,
    /// Workspace theo project profile (M21.1), tra cứu theo tên project.
    ///
    /// Rỗng ⇒ mọi project dùng chung [`Self::workspace`]. Có dữ liệu ⇒ mỗi project có
    /// thư mục riêng (và `MEMORY.md`/`USER.md` riêng), nên hai project không lẫn bộ nhớ.
    project_workspaces: BTreeMap<String, Arc<dyn WorkspaceFs>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.names())
            .finish()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// Registry rỗng.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            workspace: None,
            project_workspaces: BTreeMap::new(),
        }
    }

    /// Tạo registry với workspace.
    #[must_use]
    pub fn with_workspace(workspace: Arc<dyn WorkspaceFs>) -> Self {
        Self {
            tools: BTreeMap::new(),
            workspace: Some(workspace),
            project_workspaces: BTreeMap::new(),
        }
    }

    /// Gắn workspace cho một project profile (M21.1).
    ///
    /// Project không có trong map thì rơi về [`Self::workspace`] (project `default`).
    pub fn set_project_workspace(&mut self, project: &str, workspace: Arc<dyn WorkspaceFs>) {
        self.project_workspaces
            .insert(project.to_string(), workspace);
    }

    /// Workspace của một project profile (M21.1).
    ///
    /// `None` chỉ khi registry không gắn workspace **và** project cũng không có riêng.
    #[must_use]
    pub fn workspace_for(&self, project: &str) -> Option<Arc<dyn WorkspaceFs>> {
        self.project_workspaces
            .get(project)
            .cloned()
            .or_else(|| self.workspace.clone())
    }

    /// Trả về workspace liên kết với registry (`None` khi registry rỗng / chưa gắn).
    #[must_use]
    pub fn workspace_opt(&self) -> Option<Arc<dyn WorkspaceFs>> {
        self.workspace.clone()
    }

    /// Trả về workspace liên kết với registry.
    ///
    /// # Errors
    /// [`ToolError::Internal`] khi registry chưa được gắn workspace
    /// (dựng bằng [`ToolRegistry::new`] mà chưa gọi `with_workspace`).
    pub fn workspace(&self) -> Result<Arc<dyn WorkspaceFs>, ToolError> {
        self.workspace_opt()
            .ok_or_else(|| ToolError::Internal("registry chưa gắn workspace".into()))
    }

    /// Đăng ký tool; lỗi [`ToolError::DuplicateName`] khi trùng tên.
    ///
    /// # Errors
    /// [`ToolError::DuplicateName`].
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.spec().name;
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateName(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Tra tool theo tên.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Danh sách mô tả (sort theo tên) gửi cho model.
    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|t| t.spec()).collect()
    }

    /// Danh sách tool mà `perms` được phép thấy, **đã sort theo tên** (M21.5).
    ///
    /// Đây là *áp dụng* quyết định RBAC đã resolve ở Router, không phải chỗ ra quyết định:
    /// toàn bộ ngữ nghĩa nằm trong [`bean_types::RolePermissions::allows`].
    #[must_use]
    pub fn specs_visible_to(&self, perms: &RolePermissions) -> Vec<ToolSpec> {
        self.tools
            .values()
            .filter(|tool| perms.allows(&tool.required_tags(), &tool.also_visible_to()))
            .map(|tool| tool.spec())
            .collect()
    }

    /// Role này có được gọi tool này không? (dùng để chặn ở tầng thực thi — M21.5)
    ///
    /// Cùng ngữ nghĩa với [`Self::specs_visible_to`]: cùng một [`RolePermissions`] và cùng
    /// một hàm `allows`, nên không thể lệch nhau giữa lúc lọc payload và lúc chạy.
    #[must_use]
    pub fn allows(&self, name: &str, perms: &RolePermissions) -> bool {
        self.tools
            .get(name)
            .is_some_and(|tool| perms.allows(&tool.required_tags(), &tool.also_visible_to()))
    }

    /// Tên các tool đã đăng ký (sort).
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Số tool đã đăng ký.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Registry có rỗng không?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;

    use async_trait::async_trait;
    use bean_types::{Risk, ToolSpec};

    use crate::ctx::ToolCtx;
    use crate::error::ToolError;
    use crate::registry::ToolRegistry;
    use crate::tool::Tool;

    struct Dummy {
        name: &'static str,
        risk: Risk,
    }

    #[async_trait]
    impl Tool for Dummy {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(self.name, "tool thử", serde_json::json!({"type": "object"}))
        }
        fn risk(&self, _args: &serde_json::Value) -> Risk {
            self.risk
        }
        async fn call(
            &self,
            _ctx: &ToolCtx,
            _args: serde_json::Value,
        ) -> Result<String, ToolError> {
            Ok("ok".to_string())
        }
    }

    fn tool(name: &'static str, risk: Risk) -> Arc<dyn Tool> {
        Arc::new(Dummy { name, risk })
    }

    #[test]
    fn register_get_and_specs_are_sorted() {
        let mut registry = ToolRegistry::new();
        registry
            .register(tool("write_file", Risk::Confirm))
            .unwrap();
        registry.register(tool("read_file", Risk::Safe)).unwrap();

        let names = registry.names();
        assert_eq!(names, vec!["read_file", "write_file"]);
        let specs = registry.specs();
        assert_eq!(specs[0].name, "read_file");
        assert!(registry.get("read_file").is_some());
        assert!(registry.get("nope").is_none());
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn duplicate_registration_is_an_error() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("read_file", Risk::Safe)).unwrap();
        let err = registry
            .register(tool("read_file", Risk::Safe))
            .unwrap_err();
        assert!(matches!(err, ToolError::DuplicateName(_)), "{err:?}");
    }
}
