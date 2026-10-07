//! Factory Pattern - Dependency Injection (DIP)
//!
//! This module provides factory patterns for creating agent components
//! with proper dependency injection following DIP.

use std::sync::Arc;

use crate::store::Store;
use crate::solid::agent::{AgentContext, AgentPolicy, AgentLogger, AgentExecutor};
use crate::solid::tool::{ToolRegistry, ToolFactory};
use crate::solid::channel::{ChannelRegistry, ChannelAdapterFactory};
use bean_types::Config;
use bean_security::policy::SessionPolicy;

/// Factory for creating agent components
pub struct AgentFactory {
    config: Config,
}

impl AgentFactory {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Create an agent context
    pub fn create_agent_context<'a>(
        &self,
        store: &'a dyn Store,
        session_id: i64,
    ) -> AgentContext<'a> {
        AgentContext::new(&self.config, store, session_id)
    }

    /// Create an agent policy
    pub fn create_agent_policy<'a>(
        &self,
        permissions: &'a bean_types::RolePermissions,
        channel: &'a str,
    ) -> AgentPolicy<'a> {
        AgentPolicy::new(permissions, channel)
    }

    /// Create an agent logger
    pub fn create_agent_logger(
        &self,
        audit: Option<Arc<bean_security::AuditLog>>,
    ) -> AgentLogger {
        AgentLogger::new(audit)
    }

    /// Create an agent executor
    pub fn create_agent_executor<'a>(
        &self,
        context: AgentContext<'a>,
        policy: AgentPolicy<'a>,
        logger: AgentLogger,
    ) -> AgentExecutor<'a> {
        AgentExecutor::new(context, policy, logger)
    }
}

/// Factory for creating tool registries
pub struct ToolRegistryFactory;

impl ToolRegistryFactory {
    /// Create a new empty tool registry
    pub fn create_empty() -> ToolRegistry {
        ToolRegistry::new()
    }

    /// Create a tool registry with default built-in tools
    pub fn create_with_defaults() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        // Register default tools here
        // registry.register(BuiltinToolFactory::new());
        registry
    }

    /// Create a tool registry from a list of factories
    pub fn create_from_factories(factories: Vec<Box<dyn ToolFactory>>) -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        for factory in factories {
            registry.register(factory);
        }
        registry
    }
}

/// Factory for creating channel adapters
pub struct ChannelAdapterFactoryFactory;

impl ChannelAdapterFactoryFactory {
    /// Create a channel registry with all default adapters
    pub fn create_default_registry() -> ChannelRegistry {
        let registry = ChannelRegistry::new();
        // Register default channel factories
        // registry.register(TelegramAdapterFactory::new());
        // registry.register(WebAdapterFactory::new());
        // registry.register(CliAdapterFactory::new());
        registry
    }
}

/// Main composition root for the entire application
pub struct CompositionRoot {
    config: Config,
}

impl CompositionRoot {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Create the agent factory
    pub fn create_agent_factory(&self) -> AgentFactory {
        AgentFactory::new(self.config.clone())
    }

    /// Create the tool registry factory
    pub fn create_tool_registry_factory(&self) -> ToolRegistryFactory {
        ToolRegistryFactory
    }

    /// Create the channel registry factory
    pub fn create_channel_registry_factory(&self) -> ChannelAdapterFactoryFactory {
        ChannelAdapterFactoryFactory
    }

    /// Build the complete agent system
    pub fn build_agent_system(
        &self,
        store: Arc<dyn Store>,
        tool_registry: ToolRegistry,
        channel_registry: ChannelRegistry,
    ) -> AgentSystem {
        AgentSystem::new(
            self.config.clone(),
            store,
            tool_registry,
            channel_registry,
        )
    }
}

/// Main agent system composed of all components
pub struct AgentSystem {
    config: Config,
    store: Arc<dyn Store>,
    tool_registry: ToolRegistry,
    channel_registry: ChannelRegistry,
}

impl AgentSystem {
    pub fn new(
        config: Config,
        store: Arc<dyn Store>,
        tool_registry: ToolRegistry,
        channel_registry: ChannelRegistry,
    ) -> Self {
        Self {
            config,
            store,
            tool_registry,
            channel_registry,
        }
    }

    /// Get the config
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get the store
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Get the tool registry
    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    /// Get the channel registry
    pub fn channel_registry(&self) -> &ChannelRegistry {
        &self.channel_registry
    }
}
