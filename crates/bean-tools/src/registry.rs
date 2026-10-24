//! `ToolRegistry` — sổ đăng ký tool cho một tiến trình agent (agents.md mục 7.1).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use bean_types::{Risk, RolePermissions, ToolSpec};

use crate::error::ToolError;
use crate::tool::Tool;
use crate::workspace::WorkspaceFs;

/// Quản lý workspace và project profiles (M21.1).
#[derive(Default, Clone)]
pub struct WorkspaceRegistry {
    /// Workspace gốc dùng cho tool (M4: `CapWorkspace` trên cap-std — bean-security).
    workspace: Option<Arc<dyn WorkspaceFs>>,
    /// Workspace theo project profile, tra cứu theo tên project.
    ///
    /// Rỗng ⇒ mọi project dùng chung [`Self::workspace`]. Có dữ liệu ⇒ mỗi project có
    /// thư mục riêng (và `MEMORY.md`/`USER.md` riêng), nên hai project không lẫn bộ nhớ.
    project_workspaces: BTreeMap<String, Arc<dyn WorkspaceFs>>,
}

impl WorkspaceRegistry {
    /// Registry rỗng.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Tạo registry với workspace gốc.
    #[must_use]
    pub fn with_workspace(workspace: Arc<dyn WorkspaceFs>) -> Self {
        Self {
            workspace: Some(workspace),
            project_workspaces: BTreeMap::new(),
        }
    }

    /// Gắn workspace cho một project profile (M21.1).
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

    /// Trả về workspace gốc (`None` khi chưa gắn).
    #[must_use]
    pub fn workspace_opt(&self) -> Option<Arc<dyn WorkspaceFs>> {
        self.workspace.clone()
    }
}

/// Registry lưu `Arc<dyn Tool>` theo tên; `specs()` trả về **đã sort theo tên** để
/// prompt/schema gửi model ổn định giữa các lượt (không nhảy thứ tự ngẫu nhiên).
///
/// Tool bị tắt runtime (toggle từ UI) nằm trong `disabled` — **interior mutability**
/// vì registry sống sau `Arc<ToolRegistry>` dùng chung bởi Router/web server; khoá
/// chỉ giữ trong phạm vi đồng bộ, không bao giờ qua `.await` (agents.md mục 22.7).
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    workspaces: WorkspaceRegistry,
    disabled: RwLock<BTreeSet<String>>,
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
            workspaces: WorkspaceRegistry::new(),
            disabled: RwLock::new(BTreeSet::new()),
        }
    }

    /// Tạo registry với workspace.
    #[must_use]
    pub fn with_workspace(workspace: Arc<dyn WorkspaceFs>) -> Self {
        Self {
            tools: BTreeMap::new(),
            workspaces: WorkspaceRegistry::with_workspace(workspace),
            disabled: RwLock::new(BTreeSet::new()),
        }
    }

    /// Trả về tham chiếu đến quản lý workspace.
    #[must_use]
    pub fn workspaces(&self) -> &WorkspaceRegistry {
        &self.workspaces
    }

    /// Trả về tham chiếu mutable đến quản lý workspace.
    pub fn workspaces_mut(&mut self) -> &mut WorkspaceRegistry {
        &mut self.workspaces
    }

    /// Gắn workspace cho một project profile (M21.1).
    ///
    /// Project không có trong map thì rơi về workspace gốc (project `default`).
    pub fn set_project_workspace(&mut self, project: &str, workspace: Arc<dyn WorkspaceFs>) {
        self.workspaces.set_project_workspace(project, workspace);
    }

    /// Workspace của một project profile (M21.1).
    #[must_use]
    pub fn workspace_for(&self, project: &str) -> Option<Arc<dyn WorkspaceFs>> {
        self.workspaces.workspace_for(project)
    }

    /// Trả về workspace gốc (`None` khi chưa gắn).
    #[must_use]
    pub fn workspace_opt(&self) -> Option<Arc<dyn WorkspaceFs>> {
        self.workspaces.workspace_opt()
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
    ///
    /// Tool bị tắt runtime (toggle từ UI) bị **ẩn khỏi payload** — model không nên
    /// thấy tool mà lời gọi sẽ bị chặn ở tầng thực thi. Danh sách này vẫn giữ tool
    /// trong `names()`/`get()` để UI nắm được toàn bộ registry và bật lại được.
    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        let disabled = self.snapshot_disabled();
        self.tools
            .iter()
            .filter(|(name, _tool)| !disabled.contains(name.as_str()))
            .map(|(_name, tool)| tool.spec())
            .collect()
    }

    /// Danh sách tool mà `perms` được phép thấy, **đã sort theo tên** (M21.5).
    ///
    /// Đây là *áp dụng* quyết định RBAC đã resolve ở Router, không phải chỗ ra quyết định:
    /// toàn bộ ngữ nghĩa nằm trong [`bean_types::RolePermissions::allows`].
    ///
    /// (M27) Với lượt voice (`perms.safe_only`), mọi tool có mức rủi ro cơ bản lớn hơn
    /// `Safe` bị ẩn khỏi payload model — model không wasting lượt gọi thứ bị chặn ở tầng
    /// thực thi. Dùng `risk(&Value::Null)` (mức khởi điểm) đúng như `ToolDto`, nhất quán
    /// với `gate::expose_gate`.
    #[must_use]
    pub fn specs_visible_to(&self, perms: &RolePermissions) -> Vec<ToolSpec> {
        let disabled = self.snapshot_disabled();
        self.tools
            .iter()
            .filter(|(name, tool)| {
                if disabled.contains(name.as_str()) {
                    return false;
                }
                let access = tool.access();
                if perms.safe_only && tool.risk(&serde_json::Value::Null) != Risk::Safe {
                    return false;
                }
                perms.allows(&access.required_tags, &access.also_visible_to)
            })
            .map(|(_name, tool)| tool.spec())
            .collect()
    }

    /// Bật/tắt tool tại runtime; trả `false` khi tool không tồn tại trong registry.
    ///
    /// Chỉ đổi trạng thái trong RAM — persistence thuộc store (`tool_overrides`);
    /// caller (REST handler) phải lưu DB **trước** rồi mới gọi hàm này.
    pub fn set_tool_enabled(&self, name: &str, enabled: bool) -> bool {
        if !self.tools.contains_key(name) {
            return false;
        }
        if let Ok(mut disabled) = self.disabled.write() {
            if enabled {
                disabled.remove(name);
            } else {
                disabled.insert(name.to_string());
            }
            return true;
        }
        // Khoá hỏng (poisoned bởi panic trong vùng khoá) — trạng thái registry
        // không thể chắc chắn nữa, từ chối thay đổi thay vì làm sai thêm.
        false
    }

    /// Tool này đang bị tắt runtime không? Dùng ở tầng thực thi (gate cuối).
    #[must_use]
    pub fn is_disabled(&self, name: &str) -> bool {
        self.snapshot_disabled().contains(name)
    }

    /// Tên các tool đang bị tắt (sort).
    #[must_use]
    pub fn disabled_names(&self) -> Vec<String> {
        self.snapshot_disabled().into_iter().collect()
    }

    /// Nạp hàng loạt danh sách tool bị tắt (lúc khởi động, từ store).
    pub fn set_disabled_all<I>(&self, names: I)
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        if let Ok(mut disabled) = self.disabled.write() {
            disabled.clear();
            disabled.extend(names.into_iter().map(Into::into));
        }
    }

    /// Bản chụp danh sách tool bị tắt — khoá lấy và thả trong cùng hàm đồng bộ.
    /// Khoá hỏng (poisoned) coi như rỗng: registry không rơi vào trạng thái "tắt hết".
    fn snapshot_disabled(&self) -> BTreeSet<String> {
        self.disabled
            .read()
            .ok()
            .map(|set| set.clone())
            .unwrap_or_default()
    }

    /// Role này có được gọi tool này không? (dùng để chặn ở tầng thực thi — M21.5)
    ///
    /// Cùng ngữ nghĩa với [`Self::specs_visible_to`]: cùng một [`RolePermissions`] và cùng
    /// một hàm `allows`, nên không thể lệch nhau giữa lúc lọc payload và lúc chạy.
    #[must_use]
    pub fn allows(&self, name: &str, perms: &RolePermissions) -> bool {
        self.tools.get(name).is_some_and(|tool| {
            let access = tool.access();
            perms.allows(&access.required_tags, &access.also_visible_to)
        })
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

    #[test]
    fn disabled_tool_is_hidden_from_specs_but_still_registered() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("read_file", Risk::Safe)).unwrap();
        registry
            .register(tool("write_file", Risk::Confirm))
            .unwrap();

        assert!(registry.set_tool_enabled("write_file", false));
        // Ẩn khỏi payload model...
        let names: Vec<_> = registry.specs().into_iter().map(|spec| spec.name).collect();
        assert_eq!(names, vec!["read_file"]);
        // ...nhưng vẫn tồn tại trong registry để UI bật lại được.
        assert!(registry.get("write_file").is_some());
        assert!(registry.names().contains(&"write_file".to_string()));
        assert_eq!(registry.len(), 2);
        assert!(registry.is_disabled("write_file"));
        assert_eq!(registry.disabled_names(), vec!["write_file".to_string()]);
    }

    #[test]
    fn enable_again_removes_override() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("read_file", Risk::Safe)).unwrap();
        registry.set_tool_enabled("read_file", false);
        assert!(registry.is_disabled("read_file"));
        assert!(registry.set_tool_enabled("read_file", true));
        assert!(!registry.is_disabled("read_file"));
        assert!(registry.disabled_names().is_empty());
    }

    #[test]
    fn toggle_unknown_tool_returns_false() {
        let registry = ToolRegistry::new();
        assert!(!registry.set_tool_enabled("nope", false));
        assert!(!registry.is_disabled("nope"));
    }

    #[test]
    fn set_disabled_all_replaces_previous_state() {
        let mut registry = ToolRegistry::new();
        registry.register(tool("a", Risk::Safe)).unwrap();
        registry.register(tool("b", Risk::Safe)).unwrap();
        registry.register(tool("c", Risk::Safe)).unwrap();
        registry.set_tool_enabled("a", false);
        registry.set_disabled_all(["b", "c"]);
        assert!(!registry.is_disabled("a"), "phải thay thế, không cộng dồn");
        assert!(registry.is_disabled("b"));
        assert!(registry.is_disabled("c"));
    }
}
