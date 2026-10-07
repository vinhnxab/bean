//! Tool Factory Pattern - Open/Closed Principle (OCP) + Dependency Inversion (DIP)
//!
//! # Mục tiêu
//!
//! Enable extension của tool system mà không cần sửa existing code:
//! - Thêm tool mới chỉ cần register factory
//! - ToolRegistry giữ danh sách factories
//! - Factory tạo tools khi cần (lazy instantiation)

use std::collections::HashMap;

use crate::run_io::RunIo;
use bean_tools::{ToolCtx, ToolError};
use bean_types::{ToolSpec, RolePermissions};
use tokio_util::sync::CancellationToken;

/// Base trait cho tất cả tools - extension point cho OCP
pub trait Tool: Send + Sync + 'static {
    /// Get the tool specification
    fn spec(&self) -> ToolSpec;

    /// Execute the tool
    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError>;

    /// Default implementation
    fn is_background_compatible(&self) -> bool {
        true
    }
}

/// Factory trait - OCP: Open for extension
pub trait ToolFactory: Send + Sync + 'static {
    /// Get the tool name
    fn name(&self) -> &str;

    /// Create a new tool instance
    fn create(&self, ctx: ToolCtx) -> Box<dyn Tool>;
}

/// Registry that manages tool factories - DIP: Dependencies injected via traits
pub struct ToolRegistry {
    factories: HashMap<String, Box<dyn ToolFactory>>,
}

impl ToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a new tool factory - OCP: Open for extension
    pub fn register<F>(&mut self, factory: F) -> &mut Self
    where
        F: ToolFactory + 'static,
    {
        let name = factory.name().to_string();
        self.factories.insert(name, Box::new(factory));
        self
    }

    /// Create a tool by name
    pub fn create(&self, name: &str, ctx: ToolCtx) -> Option<Box<dyn Tool>> {
        self.factories.get(name).map(|f| f.create(ctx))
    }

    /// List all available tool names
    pub fn list_tools(&self) -> Vec<&str> {
        self.factories.keys().map(|s| s.as_str()).collect()
    }

    /// Get the number of registered tools
    pub fn len(&self) -> usize {
        self.factories.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
