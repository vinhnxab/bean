//! Tool Abstraction Layer - Open/Closed Principle (OCP)
//!
//! This module provides a flexible tool system that's open for extension
//! but closed for modification. New tools can be added without changing
//! existing code.

use std::collections::HashMap;

use crate::run_io::RunIo;
use bean_tools::{ToolCtx, ToolError};
use bean_types::{ToolSpec, RolePermissions};

/// Factory trait for creating tools
pub trait ToolFactory: Send + Sync + 'static {
    fn tool_name(&self) -> &str;
    fn create(&self, ctx: ToolCtx) -> Box<dyn Tool>;
}

/// Base trait for all tools with default implementations
pub trait Tool: Send + Sync + 'static {
    /// Get the tool specification
    fn spec(&self) -> ToolSpec;

    /// Execute the tool
    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<String, ToolError>;

    /// Default implementation that can be overridden
    fn is_background_compatible(&self) -> bool {
        true
    }
}

/// Registry that manages tool factories
pub struct ToolRegistry {
    factories: HashMap<String, Box<dyn ToolFactory>>,
}

impl ToolRegistry {
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
        let name = factory.tool_name().to_string();
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

/// Strategy pattern for tool execution policies
pub trait ToolExecutionPolicy: Send + Sync + 'static {
    fn can_execute(&self, tool_name: &str, args: &serde_json::Value) -> bool;
}

/// Default execution policy - allows all safe tools
pub struct DefaultExecutionPolicy;

impl ToolExecutionPolicy for DefaultExecutionPolicy {
    fn can_execute(&self, _tool_name: &str, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Confirmation-required policy - requires confirmation for non-safe tools
pub struct ConfirmationExecutionPolicy {
    confirmed_tools: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl ConfirmationExecutionPolicy {
    pub fn new() -> Self {
        Self {
            confirmed_tools: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn confirm_tool(&self, tool_name: &str) {
        self.confirmed_tools
            .lock()
            .unwrap()
            .push(tool_name.to_string());
    }
}

impl ToolExecutionPolicy for ConfirmationExecutionPolicy {
    fn can_execute(&self, tool_name: &str, _args: &serde_json::Value) -> bool {
        let confirmed = self.confirmed_tools.lock().unwrap();
        confirmed.contains(&tool_name.to_string())
    }
}
