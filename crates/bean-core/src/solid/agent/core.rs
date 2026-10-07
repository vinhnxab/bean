//! Agent Core - Single Responsibility Principle (SRP)
//!
//! This module contains the core agent logic split into focused components:
//! - AgentContext: Manages context building and memory
//! - AgentPolicy: Handles security policies and permissions
//! - AgentLogger: Manages audit logging
//! - AgentExecutor: Executes the agent loop

use std::sync::Arc;

use crate::store::Store;
use crate::run_io::RunIo;
use bean_types::{Config, RolePermissions};
use crate::context::TurnContext;

/// Manages agent context building and memory access
pub struct AgentContext<'a> {
    config: &'a Config,
    store: &'a dyn Store,
    session_id: i64,
}

impl<'a> AgentContext<'a> {
    pub fn new(config: &'a Config, store: &'a dyn Store, session_id: i64) -> Self {
        Self {
            config,
            store,
            session_id,
        }
    }

    /// Build context for the agent from history and memories
    pub async fn build_context(&self) -> Result<TurnContext, crate::ContextError> {
        TurnContext::build(self.store, self.session_id, self.config)
            .await
            .map_err(|e| crate::ContextError::BuildFailed(e.to_string()))
    }
}

/// Manages security policies and permissions
pub struct AgentPolicy<'a> {
    permissions: &'a RolePermissions,
    channel: &'a str,
}

impl<'a> AgentPolicy<'a> {
    pub fn new(
        permissions: &'a RolePermissions,
        channel: &'a str,
    ) -> Self {
        Self {
            permissions,
            channel,
        }
    }

    /// Check if an action is allowed based on policy
    pub fn check_policy(&self, tool_name: &str) -> bool {
        // Simplified - actual policy checking in bean_security
        self.permissions.allow_tool(tool_name)
    }
}

/// Manages audit logging
pub struct AgentLogger {
    audit: Option<Arc<bean_security::AuditLog>>,
}

impl AgentLogger {
    pub fn new(audit: Option<Arc<bean_security::AuditLog>>) -> Self {
        Self { audit }
    }

    /// Log an audit entry
    pub fn log(&self, entry: bean_security::audit::AuditEntry) {
        if let Some(ref log) = self.audit {
            log.record(entry);
        }
    }
}

/// Main agent execution loop
pub struct AgentExecutor<'a> {
    context: AgentContext<'a>,
    policy: AgentPolicy<'a>,
    logger: AgentLogger,
}

impl<'a> AgentExecutor<'a> {
    pub fn new(
        context: AgentContext<'a>,
        policy: AgentPolicy<'a>,
        logger: AgentLogger,
    ) -> Self {
        Self {
            context,
            policy,
            logger,
        }
    }

    /// Execute one turn of the agent loop
    pub async fn run_turn(
        &self,
        store: &'a dyn Store,
        user_text: String,
        io: Arc<dyn RunIo>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<String, crate::AgentError> {
        // This would be implemented with the actual agent loop
        // For now, this is a placeholder showing the structure
        todo!("Implement agent execution loop")
    }
}
